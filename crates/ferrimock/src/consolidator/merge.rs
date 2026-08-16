//! Whether a group of recordings is safe to merge into one mock.
//!
//! Consolidation's central bet is that several recordings describe one endpoint
//! and can be replaced by a single templated mock. When the bet is right the
//! collection shrinks and answers the same; when it is wrong the merged mock is
//! wrong for every member of its own group.
//!
//! The engine has always decided this by counting: merge a group of three or
//! more, keep anything smaller as it was recorded. Size is a weak proxy -- three
//! recordings of one resource merge safely, three recordings that happen to
//! share a path shape do not -- but it is cheap and it never needed data.
//!
//! [`MergeScorer`] is where a better answer plugs in. It is handed a
//! [`MergeCandidate`], which measures what the group actually looks like, and
//! answers with the probability that merging it holds. Returning `None` declines
//! and leaves the size rule in charge, so a scorer only has to speak up about
//! the groups it understands.
//!
//! The labels for training such a scorer are free, which is why
//! [`crate::consolidator::fidelity`] was built first: merge a group, replay the
//! recording through the result, and read off whether fidelity held.

// `MergeScorer::name` returns `&str`, not `&'static str`, so a scorer loaded
// from an artifact can name itself after the model it came from. Scorers that
// answer with a literal look needlessly bound as a result.
#![allow(clippy::unnecessary_literal_bound)]

use crate::config::MockConfig;
use crate::profile::ConsolidationProfile;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;

/// Pagination field names understood without a profile to say otherwise.
const BUILTIN_PAGINATION_NAMES: [&str; 10] = [
    "offset",
    "limit",
    "page",
    "per_page",
    "cursor",
    "next",
    "next_cursor",
    "has_more",
    "total",
    "total_count",
];

/// A group of recordings being considered for merging, and what can be measured
/// about it.
///
/// Every measurement is taken on demand rather than cached: a scorer that only
/// looks at size should not pay for parsing every response body.
///
/// Note that by the time the consolidator asks, the group has already been split
/// by status and response shape, so [`Self::distinct_statuses`] and
/// [`Self::response_shape_agreement`] read as uniform at that call site. They
/// measure what they say for a candidate built anywhere else.
pub struct MergeCandidate<'a> {
    group: &'a [MockConfig],
    profile: &'a dyn ConsolidationProfile,
}

impl<'a> MergeCandidate<'a> {
    pub fn new(group: &'a [MockConfig], profile: &'a dyn ConsolidationProfile) -> Self {
        Self { group, profile }
    }

    /// The recordings under consideration.
    pub fn group(&self) -> &'a [MockConfig] {
        self.group
    }

    /// How many recordings would be merged.
    pub fn size(&self) -> usize {
        self.group.len()
    }

    /// How many distinct URLs the group covers. Equal to [`Self::size`] when
    /// every recording hit a different path.
    pub fn distinct_urls(&self) -> usize {
        self.group
            .iter()
            .filter_map(url_of)
            .collect::<FxHashSet<_>>()
            .len()
    }

    /// How many path positions took more than one value across the group. These
    /// are the positions a merged mock has to turn into placeholders.
    pub fn varying_segments(&self) -> usize {
        self.segment_columns()
            .iter()
            .filter(|column| column.len() > 1)
            .count()
    }

    /// The most values any single path position took.
    pub fn max_distinct_per_segment(&self) -> usize {
        self.segment_columns()
            .iter()
            .map(FxHashSet::len)
            .max()
            .unwrap_or(0)
    }

    /// Mean normalised entropy of the varying path positions, in `[0, 1]`.
    ///
    /// A position holding a different id in every recording scores 1.0 and is
    /// almost certainly a placeholder. A position alternating between two spellings
    /// of a collection name scores low, and merging across it is how two endpoints
    /// become one wrong mock.
    #[allow(clippy::cast_precision_loss)] // group sizes are far below f64's integer range
    pub fn segment_entropy(&self) -> f64 {
        let size = self.size();
        if size < 2 {
            return 0.0;
        }
        let ceiling = (size as f64).log2();
        if ceiling <= 0.0 {
            return 0.0;
        }

        let mut varying = 0_usize;
        let mut total = 0.0_f64;
        for column in self.segment_counts() {
            if column.len() < 2 {
                continue;
            }
            varying += 1;
            let observed: usize = column.values().sum();
            if observed == 0 {
                continue;
            }
            let entropy: f64 = column
                .values()
                .map(|count| {
                    let share = *count as f64 / observed as f64;
                    -share * share.log2()
                })
                .sum();
            total += entropy / ceiling;
        }

        if varying == 0 {
            return 0.0;
        }
        // Summing per-position ratios drifts past 1.0 in the last bits, and the
        // range is part of what this promises its callers.
        (total / varying as f64).clamp(0.0, 1.0)
    }

    /// How many distinct status codes the group recorded.
    pub fn distinct_statuses(&self) -> usize {
        self.group
            .iter()
            .map(|mock| {
                mock.response_config
                    .as_ref()
                    .and_then(crate::config::ResponseConfig::status)
            })
            .collect::<FxHashSet<_>>()
            .len()
    }

    /// Mean pairwise Jaccard overlap of the top-level response field names, in
    /// `[0, 1]`. 1.0 means every recording answered with the same shape.
    pub fn response_shape_agreement(&self) -> f64 {
        let shapes: Vec<FxHashSet<String>> = self
            .group
            .iter()
            .map(|mock| top_level_keys(response_json(mock).as_ref()))
            .collect();
        mean_pairwise_jaccard(&shapes)
    }

    /// Mean pairwise Jaccard overlap of the response header names, in `[0, 1]`.
    pub fn header_agreement(&self) -> f64 {
        let headers: Vec<FxHashSet<String>> = self
            .group
            .iter()
            .map(|mock| {
                mock.response_config
                    .as_ref()
                    .and_then(crate::config::ResponseConfig::headers)
                    .map(|headers| headers.keys().map(|key| key.to_lowercase()).collect())
                    .unwrap_or_default()
            })
            .collect();
        mean_pairwise_jaccard(&headers)
    }

    /// The share of request-body matchers that not every member pins, in
    /// `[0, 1]`. Zero when no member pins a body at all.
    ///
    /// A group whose members were told apart by their POST bodies loses that
    /// distinction when merged: the surviving mock answers the first body it
    /// sees for all of them.
    #[allow(clippy::cast_precision_loss)] // matcher counts are small
    pub fn request_body_divergence(&self) -> f64 {
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        for mock in self.group {
            if let Some(match_config) = mock.match_config.as_ref() {
                seen.extend(match_config.body.keys().map(String::as_str));
            }
        }
        if seen.is_empty() {
            return 0.0;
        }

        let shared = seen
            .iter()
            .filter(|key| {
                self.group.iter().all(|mock| {
                    mock.match_config
                        .as_ref()
                        .is_some_and(|match_config| match_config.body.contains_key(**key))
                })
            })
            .count();

        1.0 - (shared as f64 / seen.len() as f64)
    }

    /// Whether anything in the group looks like paging -- a pagination query
    /// parameter, or a pagination field in a response.
    ///
    /// Paged recordings of one endpoint differ only by their cursor, which makes
    /// them look ideal to merge and makes the merged mock answer page one to
    /// every page request.
    pub fn pagination_evidence(&self) -> bool {
        let dialect = self.profile.pagination_dialect();
        let known = |name: &str| {
            let lowered = name.to_lowercase();
            if BUILTIN_PAGINATION_NAMES.contains(&lowered.as_str()) {
                return true;
            }
            dialect.is_some_and(|dialect| {
                [
                    &dialect.total,
                    &dialect.offset,
                    &dialect.limit,
                    &dialect.next,
                    &dialect.prev,
                    &dialect.has_more,
                ]
                .into_iter()
                .flatten()
                .any(|candidate| candidate.eq_ignore_ascii_case(&lowered))
            })
        };

        self.group.iter().any(|mock| {
            let in_query = mock
                .match_config
                .as_ref()
                .is_some_and(|match_config| match_config.query.keys().any(|key| known(key)));
            let in_url = url_of(mock).is_some_and(|url| {
                url.split_once('?').is_some_and(|(_, query)| {
                    query
                        .split('&')
                        .filter_map(|pair| pair.split('=').next())
                        .any(known)
                })
            });
            let in_response = top_level_keys(response_json(mock).as_ref())
                .iter()
                .any(|key| known(key));

            in_query || in_url || in_response
        })
    }

    /// The distinct values seen at each path position.
    fn segment_columns(&self) -> Vec<FxHashSet<&'a str>> {
        let mut columns: Vec<FxHashSet<&'a str>> = Vec::new();
        for mock in self.group {
            let Some(url) = url_of(mock) else { continue };
            for (index, segment) in path_of(url).split('/').enumerate() {
                if columns.len() <= index {
                    columns.resize_with(index + 1, FxHashSet::default);
                }
                if let Some(column) = columns.get_mut(index) {
                    column.insert(segment);
                }
            }
        }
        columns
    }

    /// The value counts at each path position, for entropy.
    fn segment_counts(&self) -> Vec<FxHashMap<&'a str, usize>> {
        let mut columns: Vec<FxHashMap<&'a str, usize>> = Vec::new();
        for mock in self.group {
            let Some(url) = url_of(mock) else { continue };
            for (index, segment) in path_of(url).split('/').enumerate() {
                if columns.len() <= index {
                    columns.resize_with(index + 1, FxHashMap::default);
                }
                if let Some(column) = columns.get_mut(index) {
                    *column.entry(segment).or_insert(0) += 1;
                }
            }
        }
        columns
    }
}

/// A verdict on whether a group of recordings should become one mock.
///
/// The default implementation declines, which leaves the consolidator's size
/// rule in charge. A scorer therefore only has to answer for the groups it
/// actually knows something about.
pub trait MergeScorer: Send + Sync {
    /// Short identifier, used in diagnostics.
    fn name(&self) -> &str;

    /// The probability, in `[0, 1]`, that merging this group answers every
    /// recording in it the way the recording did.
    ///
    /// `None` declines and hands the decision back to the size rule.
    fn safe_to_merge(&self, candidate: &MergeCandidate<'_>) -> Option<f64> {
        let _ = candidate;
        None
    }
}

/// The built-in rule: a group merges once it is large enough.
///
/// This is what the engine has always done, expressed as a scorer so that
/// replacing it is a matter of passing a different one.
#[derive(Debug, Clone, Copy)]
pub struct SizeThreshold {
    pub min_group_size: usize,
}

impl MergeScorer for SizeThreshold {
    fn name(&self) -> &str {
        "size-threshold"
    }

    fn safe_to_merge(&self, candidate: &MergeCandidate<'_>) -> Option<f64> {
        Some(if candidate.size() >= self.min_group_size {
            1.0
        } else {
            0.0
        })
    }
}

/// The URL a recording matched on.
fn url_of(mock: &MockConfig) -> Option<&str> {
    let match_config = mock.match_config.as_ref()?;
    match_config
        .url
        .as_deref()
        .or_else(|| match_config.urls.first().map(String::as_str))
}

/// The path part of a URL, without scheme, host or query.
fn path_of(url: &str) -> &str {
    let without_scheme = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    let path = if url.contains("://") {
        without_scheme
    } else {
        url
    };
    path.split_once('?').map_or(path, |(path, _)| path)
}

/// A recording's response body, parsed as JSON.
fn response_json(mock: &MockConfig) -> Option<JsonValue> {
    let response = mock.response_config.as_ref()?;
    // `json` is absent as `Null` rather than as `None`, and a recording made
    // from a HAR always carries its payload in `body`.
    if let Some(json) = response.json().filter(|json| !json.is_null()) {
        return Some(json.clone());
    }
    serde_json::from_str(response.body()?).ok()
}

/// The field names of a JSON object, or nothing for any other kind of value.
fn top_level_keys(value: Option<&JsonValue>) -> FxHashSet<String> {
    value
        .and_then(JsonValue::as_object)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// Mean Jaccard overlap over every pair of sets, in `[0, 1]`.
///
/// Two empty sets agree completely: neither carried anything to disagree about.
#[allow(clippy::cast_precision_loss)] // pair counts are small
fn mean_pairwise_jaccard(sets: &[FxHashSet<String>]) -> f64 {
    if sets.len() < 2 {
        return 1.0;
    }

    let mut pairs = 0_usize;
    let mut total = 0.0_f64;
    for (index, left) in sets.iter().enumerate() {
        for right in sets.iter().skip(index + 1) {
            let union = left.union(right).count();
            let overlap = if union == 0 {
                1.0
            } else {
                left.intersection(right).count() as f64 / union as f64
            };
            total += overlap;
            pairs += 1;
        }
    }

    if pairs == 0 {
        1.0
    } else {
        (total / pairs as f64).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
mod tests {
    use super::*;
    use crate::config::{MatchConfig, ResponseConfig};
    use crate::profile::DefaultProfile;

    fn mock(id: &str, url: &str, status: u16, body: &str) -> MockConfig {
        MockConfig {
            id: id.into(),
            match_config: Some(MatchConfig {
                method: Some("GET".to_string()),
                url: Some(url.to_string()),
                ..MatchConfig::default()
            }),
            response_config: Some(ResponseConfig::Structured {
                status: Some(status),
                headers: FxHashMap::default(),
                body: Some(body.to_string()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(JsonValue::Null),
            }),
            ..MockConfig::default()
        }
    }

    fn candidate(group: &[MockConfig]) -> MergeCandidate<'_> {
        MergeCandidate::new(group, &DefaultProfile)
    }

    #[test]
    fn an_id_that_differs_every_time_is_maximally_variable() {
        let group = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("b", "/v2/files/2", 200, r#"{"id":"2"}"#),
            mock("c", "/v2/files/3", 200, r#"{"id":"3"}"#),
            mock("d", "/v2/files/4", 200, r#"{"id":"4"}"#),
        ];
        let candidate = candidate(&group);

        assert_eq!(candidate.size(), 4);
        assert_eq!(candidate.distinct_urls(), 4);
        assert_eq!(candidate.varying_segments(), 1);
        assert_eq!(candidate.max_distinct_per_segment(), 4);
        assert_eq!(
            candidate.segment_entropy(),
            1.0,
            "four values over four recordings is the most a position can vary"
        );
    }

    #[test]
    fn a_position_holding_two_collection_names_barely_varies() {
        // The kind of group that should not merge: the varying position names a
        // different resource rather than an instance of one.
        let group = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("b", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("c", "/v2/folders/1", 200, r#"{"id":"1"}"#),
            mock("d", "/v2/folders/1", 200, r#"{"id":"1"}"#),
        ];
        let candidate = candidate(&group);

        assert_eq!(candidate.varying_segments(), 1);
        assert_eq!(candidate.max_distinct_per_segment(), 2);
        assert_eq!(
            candidate.segment_entropy(),
            0.5,
            "two values evenly split over four recordings is half the ceiling"
        );
    }

    #[test]
    fn a_group_that_never_varies_has_no_entropy() {
        let group = [
            mock("a", "/v2/status", 200, "{}"),
            mock("b", "/v2/status", 200, "{}"),
        ];
        assert_eq!(candidate(&group).varying_segments(), 0);
        assert_eq!(candidate(&group).segment_entropy(), 0.0);
    }

    #[test]
    fn shapes_that_share_no_fields_do_not_agree() {
        let group = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1","name":"a"}"#),
            mock("b", "/v2/files/2", 200, r#"{"error":"gone"}"#),
        ];
        assert_eq!(candidate(&group).response_shape_agreement(), 0.0);

        let same = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1","name":"a"}"#),
            mock("b", "/v2/files/2", 200, r#"{"id":"2","name":"b"}"#),
        ];
        assert_eq!(candidate(&same).response_shape_agreement(), 1.0);
    }

    #[test]
    fn statuses_are_counted_distinctly() {
        let group = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 404, "{}"),
            mock("c", "/v2/files/3", 200, "{}"),
        ];
        assert_eq!(candidate(&group).distinct_statuses(), 2);
    }

    #[test]
    fn a_body_matcher_only_one_member_pins_reads_as_divergence() {
        let mut group = [
            mock("a", "/v2/search", 200, "{}"),
            mock("b", "/v2/search", 200, "{}"),
        ];
        if let Some(match_config) = group[0].match_config.as_mut() {
            match_config
                .body
                .insert("$.query".to_string(), JsonValue::String("cats".to_string()));
        }

        assert_eq!(
            candidate(&group).request_body_divergence(),
            1.0,
            "a pin one member carries and the other does not is lost by merging"
        );

        if let Some(match_config) = group[1].match_config.as_mut() {
            match_config
                .body
                .insert("$.query".to_string(), JsonValue::String("dogs".to_string()));
        }
        assert_eq!(
            candidate(&group).request_body_divergence(),
            0.0,
            "a pin every member carries survives the merge"
        );
    }

    #[test]
    fn no_body_matchers_anywhere_is_not_divergence() {
        let group = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
        ];
        assert_eq!(candidate(&group).request_body_divergence(), 0.0);
    }

    #[test]
    fn paging_is_visible_in_a_query_string_or_a_response() {
        let paged_url = [
            mock("a", "/v2/files?offset=0", 200, "{}"),
            mock("b", "/v2/files?offset=100", 200, "{}"),
        ];
        assert!(candidate(&paged_url).pagination_evidence());

        let paged_body = [
            mock("a", "/v2/files", 200, r#"{"entries":[],"total_count":9}"#),
            mock("b", "/v2/files", 200, r#"{"entries":[],"total_count":9}"#),
        ];
        assert!(candidate(&paged_body).pagination_evidence());

        let unpaged = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("b", "/v2/files/2", 200, r#"{"id":"2"}"#),
        ];
        assert!(!candidate(&unpaged).pagination_evidence());
    }

    #[test]
    fn a_query_string_never_reaches_the_path_columns() {
        let group = [
            mock("a", "/v2/files?offset=0", 200, "{}"),
            mock("b", "/v2/files?offset=100", 200, "{}"),
        ];
        assert_eq!(
            candidate(&group).varying_segments(),
            0,
            "the paths are identical; only the query differs"
        );
    }

    #[test]
    fn an_absolute_url_is_measured_on_its_path() {
        let group = [
            mock("a", "https://api.example.com/v2/files/1", 200, "{}"),
            mock("b", "https://api.example.com/v2/files/2", 200, "{}"),
        ];
        assert_eq!(candidate(&group).varying_segments(), 1);
        assert_eq!(candidate(&group).distinct_urls(), 2);
    }

    #[test]
    fn the_size_threshold_reproduces_the_rule_it_replaces() {
        let scorer = SizeThreshold { min_group_size: 3 };
        assert_eq!(scorer.name(), "size-threshold");

        let small = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
        ];
        assert_eq!(scorer.safe_to_merge(&candidate(&small)), Some(0.0));

        let big = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
            mock("c", "/v2/files/3", 200, "{}"),
        ];
        assert_eq!(scorer.safe_to_merge(&candidate(&big)), Some(1.0));
    }

    #[test]
    fn a_scorer_that_declines_says_nothing() {
        struct Quiet;
        impl MergeScorer for Quiet {
            fn name(&self) -> &str {
                "quiet"
            }
        }

        let group = [mock("a", "/v2/files/1", 200, "{}")];
        assert!(Quiet.safe_to_merge(&candidate(&group)).is_none());
    }
}
