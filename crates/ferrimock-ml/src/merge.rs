//! Learning when a group of recordings is safe to merge.
//!
//! Field classification needs a human to say what a field means, which is why
//! the ship gate exists and why a reviewed corpus is the bottleneck. This
//! question needs nobody: merge a group, replay the recording through the
//! result, and the fidelity harness says whether the merge held. The label is
//! not an opinion about the data -- it is the measured consequence of acting on
//! it.
//!
//! That makes the labelling loop worth running before any model exists, because
//! what it produces is useful on its own: a list of the groups the engine merges
//! today that it should not, and the groups it refuses that were safe all along.
//!
//! One group is scored at a time, against the *whole* collection. Merging in
//! isolation would miss the failure that matters most -- a merged pattern
//! swallowing requests that belonged to another group -- because cross-talk only
//! means anything at collection scope.

// `MergeScorer::name` returns `&str`, not `&'static str`, so a scorer loaded
// from an artifact can name itself after the model it came from. Scorers that
// answer with a literal look needlessly bound as a result.
#![allow(clippy::unnecessary_literal_bound)]

use ferrimock::config::{MockCollectionConfig, MockConfig};
use ferrimock::consolidator::pattern::PatternDetector;
use ferrimock::consolidator::{
    ConsolidatorOptions, FidelityOptions, MergeCandidate, MergeScorer, MockConsolidator, shape,
};
use ferrimock::profile::ConsolidationProfile;
use ferrimock::recorder::RecordedInteraction;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Bumped whenever the meaning or order of any dimension changes.
///
/// Adding a feature at the end still counts: the width is part of the contract.
pub const MERGE_FEATURE_LAYOUT_VERSION: u32 = 1;

/// Width of the vector [`features_of`] produces.
pub const MERGE_FEATURE_COUNT: usize = 11;

/// What each dimension means, in order. The layout is the contract.
pub const MERGE_FEATURE_NAMES: [&str; MERGE_FEATURE_COUNT] = [
    "log_size",
    "distinct_url_ratio",
    "varying_segments",
    "max_distinct_segment_ratio",
    "segment_entropy",
    "status_spread",
    "response_shape_agreement",
    "header_agreement",
    "request_body_divergence",
    "pagination_evidence",
    "meets_size_threshold",
];

/// Group size that saturates [`MERGE_FEATURE_NAMES`]'s `log_size`. Recordings of
/// one endpoint run to hundreds; past this the exact count stops carrying
/// information about whether the merge is safe.
const SIZE_CEILING: f64 = 64.0;

/// Varying-position count that saturates its dimension.
const SEGMENT_CEILING: f64 = 8.0;

/// The size the built-in rule merges at, offered to the model so that beating
/// the rule means beating something the model could have copied.
const BUILTIN_SIZE_THRESHOLD: usize = 3;

/// Render a candidate group as a fixed-width vector.
#[allow(clippy::cast_precision_loss)] // group sizes are far below f64's integer range
pub fn features_of(candidate: &MergeCandidate<'_>) -> Vec<f64> {
    let size = candidate.size().max(1) as f64;

    vec![
        size.log(SIZE_CEILING).clamp(0.0, 1.0),
        candidate.distinct_urls() as f64 / size,
        (candidate.varying_segments() as f64 / SEGMENT_CEILING).clamp(0.0, 1.0),
        candidate.max_distinct_per_segment() as f64 / size,
        candidate.segment_entropy(),
        f64::from(candidate.distinct_statuses() > 1),
        candidate.response_shape_agreement(),
        candidate.header_agreement(),
        candidate.request_body_divergence(),
        f64::from(candidate.pagination_evidence()),
        f64::from(candidate.size() >= BUILTIN_SIZE_THRESHOLD),
    ]
}

/// One group, measured and then labelled by what merging it actually cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeExample {
    /// The ids of the recordings in the group.
    ///
    /// These are only meaningful within the split that produced them: the
    /// collection is built from part of the recording and its mocks are
    /// numbered from one over that subset, so `har-entry-52` here is not
    /// `har-entry-52` in the full recording. Use [`Self::requests`] to identify
    /// a group.
    pub group_ids: Vec<String>,
    /// The request line of each member, as `METHOD /path`.
    ///
    /// What makes a row actionable: reading a labelled set is asking "which
    /// endpoints were these", and the ids cannot answer it.
    #[serde(default)]
    pub requests: Vec<String>,
    /// The layout [`Self::features`] was produced under.
    pub layout_version: u32,
    pub features: Vec<f64>,
    /// Whether the merge held at both levels below.
    pub safe: bool,
    /// How much behavioural fidelity the merge cost, relative to the
    /// unconsolidated recording. Negative means ground was lost.
    ///
    /// Rarely negative, and not because merging is always right: every mock the
    /// consolidator leaves alone keeps an `exact:` matcher, so a merged pattern
    /// cannot swallow a recorded request belonging to anything else. What is
    /// left to go wrong is confined to the merged group, and partitioning has
    /// already made its members agree on status and shape. On its own this
    /// separates almost nothing.
    pub behavioral_delta: f64,
    /// How much value-level fidelity the merge cost.
    ///
    /// This is the level that discriminates. A merged mock answers from a
    /// template, and whether the template reproduces what was recorded is
    /// exactly the question of whether the group was one endpoint or several.
    pub value_delta: f64,
}

impl MergeExample {
    /// Read a labelled set written by [`label_groups`].
    pub fn load(path: &str) -> Result<Vec<Self>, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("could not read {path}: {e}"))?;
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|e| format!("{path} is not a labelled set: {e}"))
            })
            .collect()
    }

    /// Write a labelled set as JSON Lines.
    pub fn save(examples: &[Self], path: &str) -> Result<(), String> {
        let mut text = String::new();
        for example in examples {
            let line = serde_json::to_string(example)
                .map_err(|e| format!("could not serialise an example: {e}"))?;
            text.push_str(&line);
            text.push('\n');
        }
        std::fs::write(path, text).map_err(|e| format!("could not write {path}: {e}"))
    }
}

/// Merges exactly one group and refuses every other.
///
/// Refusing rather than declining is the point: a declined group falls back to
/// the size rule and would merge too, and then the fidelity delta would describe
/// several merges at once.
struct MergeOnly {
    target: FxHashSet<String>,
}

impl MergeScorer for MergeOnly {
    fn name(&self) -> &str {
        "merge-only"
    }

    fn safe_to_merge(&self, candidate: &MergeCandidate<'_>) -> Option<f64> {
        let ids: FxHashSet<String> = candidate
            .group()
            .iter()
            .map(|mock| mock.id.to_string())
            .collect();
        Some(f64::from(ids == self.target))
    }
}

/// Every group the consolidator would consider merging, in the form it sees them.
///
/// This mirrors the engine's own decomposition -- group by request shape, then
/// split by what was answered -- so a labelled row describes a decision the
/// engine actually makes.
pub fn candidate_groups(
    collection: &MockCollectionConfig,
    profile: &Arc<dyn ConsolidationProfile>,
) -> Vec<Vec<MockConfig>> {
    let detector = PatternDetector::with_profile(Arc::clone(profile));
    let http: Vec<MockConfig> = collection
        .mocks
        .iter()
        .filter(|mock| mock.sse.is_none() && mock.ws.is_none())
        .cloned()
        .collect();

    let mut candidates = Vec::new();
    for group in detector.group_similar_mocks(&http) {
        // A single mock has nothing to merge with, and a group of identical
        // recordings collapses by a different path that never asks a scorer.
        if group.len() < 2 || PatternDetector::are_duplicates(&group) {
            continue;
        }
        for partition in shape::partition_by_response(&group) {
            if partition.len() >= 2 {
                candidates.push(partition);
            }
        }
    }
    candidates
}

/// How a labelling run splits a recording.
#[derive(Debug, Clone)]
pub struct MergeLabelOptions {
    /// Share of interactions held back from the mocks and used to judge the
    /// merge. The rest build the collection.
    pub holdout_ratio: f64,
    pub seed: u64,
}

impl Default for MergeLabelOptions {
    fn default() -> Self {
        Self {
            holdout_ratio: 0.3,
            seed: 0,
        }
    }
}

/// Label every candidate group by what merging it does to traffic the mocks
/// were never built from.
///
/// Replaying the recording that produced the mocks cannot answer this. Every
/// interaction in it already has its own mock, so nothing has to generalise:
/// behavioural fidelity holds because unmerged mocks keep exact matchers, and
/// value equality falls because a template does not reproduce recorded values.
/// Both outcomes are the same for a good merge and a bad one.
///
/// Holding traffic back makes the question answerable. A group that really was
/// one endpoint produces a pattern that answers requests it never saw; a group
/// that was two endpoints wearing one path shape produces a pattern that answers
/// them wrongly. The delta against the same collection unmerged is that
/// difference and nothing else, because only this group's merge is enabled.
pub async fn label_groups(
    interactions: &[RecordedInteraction],
    profile: &Arc<dyn ConsolidationProfile>,
    fidelity: &FidelityOptions,
    options: &MergeLabelOptions,
) -> Result<Vec<MergeExample>, String> {
    let (build, holdout) = split_interactions(interactions, options);
    if holdout.is_empty() {
        return Err(
            "every interaction went into the mocks, so there is no unseen traffic to judge a \
             merge on"
                .to_string(),
        );
    }

    let collection = collection_from(&build).await?;
    let candidates = candidate_groups(&collection, profile);
    let mut examples = Vec::with_capacity(candidates.len());

    for partition in candidates {
        let ids: Vec<String> = partition.iter().map(|mock| mock.id.to_string()).collect();
        let scorer = MergeOnly {
            target: ids.iter().cloned().collect(),
        };

        let mut consolidator = MockConsolidator::with_options(ConsolidatorOptions {
            profile: Arc::clone(profile),
            merge_scorer: Some(Arc::new(scorer)),
            ..ConsolidatorOptions::default()
        });

        let (_, report) = consolidator
            .consolidate_verified(&holdout, collection.clone(), fidelity)
            .await
            .map_err(|e| format!("could not verify a merge of {}: {e}", ids.join(", ")))?;

        let behavioral_delta = report.behavioral_delta();
        let value_delta = report.score.value_equal_ratio() - report.baseline.value_equal_ratio();
        let candidate = MergeCandidate::new(&partition, profile.as_ref());
        examples.push(MergeExample {
            group_ids: ids,
            requests: partition.iter().map(request_line).collect(),
            layout_version: MERGE_FEATURE_LAYOUT_VERSION,
            features: features_of(&candidate),
            // Merging is what lets a collection answer at all beyond what it
            // recorded, so a merge that generalises shows up as ground gained.
            safe: behavioral_delta >= 0.0,
            behavioral_delta,
            value_delta,
        });
    }

    Ok(examples)
}

/// A mock's request line, as `METHOD /path`.
fn request_line(mock: &MockConfig) -> String {
    let Some(match_config) = mock.match_config.as_ref() else {
        return mock.id.to_string();
    };

    let method = match_config
        .method
        .as_deref()
        .or_else(|| match_config.methods.first().map(String::as_str))
        .unwrap_or("*");
    let url = match_config
        .url
        .as_deref()
        .or_else(|| match_config.urls.first().map(String::as_str))
        .unwrap_or("");

    // The converter pins unmerged recordings with an `exact:` matcher, which
    // says how the mock matches rather than what was requested.
    format!("{method} {}", url.strip_prefix("exact:").unwrap_or(url))
}

/// Build a mock collection from a set of interactions.
async fn collection_from(
    interactions: &[RecordedInteraction],
) -> Result<MockCollectionConfig, String> {
    let mocks = ferrimock::config::har::HarLoader::new()
        .convert_interactions_to_mocks(interactions)
        .await
        .map_err(|e| format!("could not build mocks from the recording: {e}"))?;

    Ok(MockCollectionConfig {
        name: Some("labelling split".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
        world: None,
        machines: None,
    })
}

/// Split interactions into the ones that build the mocks and the ones that
/// judge them.
///
/// Shuffled first: a recording follows the order a session made its requests, so
/// taking a suffix would put whole endpoints on one side of the split and ask
/// every merge to generalise to something it never could.
fn split_interactions(
    interactions: &[RecordedInteraction],
    options: &MergeLabelOptions,
) -> (Vec<RecordedInteraction>, Vec<RecordedInteraction>) {
    let mut order: Vec<usize> = (0..interactions.len()).collect();
    shuffle(&mut order, options.seed);

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let holdout_size =
        ((interactions.len() as f64) * options.holdout_ratio.clamp(0.0, 1.0)) as usize;
    let holdout_size = holdout_size.min(interactions.len());

    let mut build = Vec::new();
    let mut holdout = Vec::new();
    for (rank, index) in order.into_iter().enumerate() {
        let Some(interaction) = interactions.get(index) else {
            continue;
        };
        if rank < holdout_size {
            holdout.push(interaction.clone());
        } else {
            build.push(interaction.clone());
        }
    }
    (build, holdout)
}

/// Knobs for fitting a merge model.
#[derive(Debug, Clone)]
pub struct MergeTrainingConfig {
    pub epochs: usize,
    pub learning_rate: f64,
    pub l2: f64,
    /// Weight each outcome by the inverse of its frequency.
    ///
    /// Most merges in a real recording are safe, and a model fitted on the raw
    /// distribution learns to say so unconditionally -- which is the size rule
    /// with extra steps.
    pub balance_outcomes: bool,
    pub seed: u64,
}

impl Default for MergeTrainingConfig {
    fn default() -> Self {
        Self {
            epochs: 300,
            learning_rate: 0.5,
            l2: 1e-4,
            balance_outcomes: true,
            seed: 0,
        }
    }
}

/// A fitted estimate of whether a merge holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeModel {
    /// Layout the weights were fitted against. Loading against a different one
    /// would silently reinterpret every dimension.
    pub feature_layout_version: u32,
    pub weights: Vec<f64>,
    pub bias: f64,
}

impl MergeModel {
    /// Fit on labelled groups.
    ///
    /// Rows from a different feature layout are refused rather than skipped: a
    /// model fitted on a mixture of layouts is wrong in a way no metric shows.
    pub fn train(examples: &[MergeExample], config: &MergeTrainingConfig) -> Result<Self, String> {
        let mut model = Self {
            feature_layout_version: MERGE_FEATURE_LAYOUT_VERSION,
            weights: vec![0.0; MERGE_FEATURE_COUNT],
            bias: 0.0,
        };

        for example in examples {
            if example.layout_version != MERGE_FEATURE_LAYOUT_VERSION {
                return Err(format!(
                    "example for {} was measured under feature layout {} but this build \
                     produces layout {}",
                    example.group_ids.join(", "),
                    example.layout_version,
                    MERGE_FEATURE_LAYOUT_VERSION
                ));
            }
            if example.features.len() != MERGE_FEATURE_COUNT {
                return Err(format!(
                    "example for {} carries {} features, not {}",
                    example.group_ids.join(", "),
                    example.features.len(),
                    MERGE_FEATURE_COUNT
                ));
            }
        }

        if examples.is_empty() {
            return Ok(model);
        }

        let (safe_weight, unsafe_weight) = outcome_weights(examples, config.balance_outcomes);
        let mut order: Vec<usize> = (0..examples.len()).collect();

        for epoch in 0..config.epochs {
            shuffle(&mut order, config.seed ^ epoch as u64);
            #[allow(clippy::cast_precision_loss)] // epoch counts are small
            let rate = config.learning_rate / (1.0 + epoch as f64 / 50.0);

            for &index in &order {
                let Some(example) = examples.get(index) else {
                    continue;
                };
                let predicted = model.probability(&example.features);
                let target = f64::from(example.safe);
                let weight = if example.safe {
                    safe_weight
                } else {
                    unsafe_weight
                };
                let error = (predicted - target) * weight * rate;
                if error == 0.0 {
                    continue;
                }
                for (w, f) in model.weights.iter_mut().zip(example.features.iter()) {
                    let decayed = (-config.l2 * rate).mul_add(*w, *w);
                    *w = (-error).mul_add(*f, decayed);
                }
                model.bias -= error;
            }
        }

        Ok(model)
    }

    /// The fitted probability that a merge holds.
    pub fn probability(&self, features: &[f64]) -> f64 {
        let logit = self.bias
            + self
                .weights
                .iter()
                .zip(features.iter())
                .map(|(w, f)| w * f)
                .sum::<f64>();
        1.0 / (1.0 + (-logit).exp())
    }

    /// What the model leans on, largest absolute weight first.
    pub fn explain(&self) -> Vec<(&'static str, f64)> {
        let mut weighted: Vec<(&'static str, f64)> = MERGE_FEATURE_NAMES
            .iter()
            .copied()
            .zip(self.weights.iter().copied())
            .collect();
        weighted.sort_by(|left, right| {
            right
                .1
                .abs()
                .partial_cmp(&left.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        weighted
    }
}

/// How a merge rule did against groups whose outcome was measured.
///
/// The two mistakes are not equally bad and are counted separately. Merging a
/// group that was not safe produces a mock that answers its own recordings
/// wrongly; refusing a safe one only leaves a collection larger than it had to
/// be. A rule is worth having when it cuts the first without giving back too
/// much of the second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Safe groups the rule merged.
    pub merged_safely: usize,
    /// Unsafe groups the rule merged. Every one of these is a broken mock.
    pub merged_unsafely: usize,
    /// Safe groups the rule refused. Reduction left on the table.
    pub refused_safely: usize,
    /// Unsafe groups the rule refused, correctly.
    pub refused_unsafely: usize,
}

impl MergeOutcome {
    pub fn total(&self) -> usize {
        self.merged_safely + self.merged_unsafely + self.refused_safely + self.refused_unsafely
    }

    /// The share of unsafe groups the rule caught, in `[0, 1]`. The number that
    /// matters: what it misses becomes a wrong mock.
    #[allow(clippy::cast_precision_loss)] // group counts are small
    pub fn unsafe_caught(&self) -> f64 {
        let total_unsafe = self.merged_unsafely + self.refused_unsafely;
        if total_unsafe == 0 {
            return 1.0;
        }
        self.refused_unsafely as f64 / total_unsafe as f64
    }

    /// The share of safe groups the rule merged, in `[0, 1]`. What the
    /// consolidation is actually for.
    #[allow(clippy::cast_precision_loss)] // group counts are small
    pub fn safe_merged(&self) -> f64 {
        let total_safe = self.merged_safely + self.refused_safely;
        if total_safe == 0 {
            return 1.0;
        }
        self.merged_safely as f64 / total_safe as f64
    }
}

/// Score a decision rule against measured outcomes.
pub fn outcome_of(
    examples: &[MergeExample],
    mut merges: impl FnMut(&MergeExample) -> bool,
) -> MergeOutcome {
    let mut outcome = MergeOutcome::default();
    for example in examples {
        match (merges(example), example.safe) {
            (true, true) => outcome.merged_safely += 1,
            (true, false) => outcome.merged_unsafely += 1,
            (false, true) => outcome.refused_safely += 1,
            (false, false) => outcome.refused_unsafely += 1,
        }
    }
    outcome
}

/// The rule in the engine today, as a predicate over a labelled row.
///
/// Reads the size-threshold dimension straight out of the feature vector, so the
/// comparison is against what the engine actually does rather than a restatement
/// of it.
pub fn size_threshold_merges(example: &MergeExample) -> bool {
    example
        .features
        .get(MERGE_FEATURE_COUNT - 1)
        .is_some_and(|meets| *meets >= 0.5)
}

/// A fitted model, as a scorer the consolidator can consult.
pub struct LearnedMergeScorer {
    model: MergeModel,
    name: String,
}

impl LearnedMergeScorer {
    pub fn new(model: MergeModel) -> Self {
        Self {
            name: format!("learned-merge-v{}", model.feature_layout_version),
            model,
        }
    }

    pub fn model(&self) -> &MergeModel {
        &self.model
    }
}

impl MergeScorer for LearnedMergeScorer {
    fn name(&self) -> &str {
        &self.name
    }

    fn safe_to_merge(&self, candidate: &MergeCandidate<'_>) -> Option<f64> {
        // A model fitted under another layout would read every dimension as
        // something else, so it declines rather than guesses.
        if self.model.feature_layout_version != MERGE_FEATURE_LAYOUT_VERSION {
            return None;
        }
        Some(self.model.probability(&features_of(candidate)))
    }
}

/// Inverse-frequency weights for the two outcomes.
#[allow(clippy::cast_precision_loss)] // group counts are small
fn outcome_weights(examples: &[MergeExample], balance: bool) -> (f64, f64) {
    if !balance {
        return (1.0, 1.0);
    }
    let safe = examples.iter().filter(|example| example.safe).count();
    let unsafe_count = examples.len() - safe;
    if safe == 0 || unsafe_count == 0 {
        return (1.0, 1.0);
    }
    let total = examples.len() as f64;
    (
        total / (2.0 * safe as f64),
        total / (2.0 * unsafe_count as f64),
    )
}

/// Deterministic shuffle, so a seed reproduces a run exactly.
fn shuffle(items: &mut [usize], seed: u64) {
    let mut state = seed | 1;
    for index in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_possible_truncation)] // modulo keeps this in range
        let swap = (state % (index as u64 + 1)) as usize;
        items.swap(index, swap);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use ferrimock::config::{MatchConfig, ResponseConfig};
    use ferrimock::profile::DefaultProfile;
    use rustc_hash::FxHashMap;
    use serde_json::Value as JsonValue;

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

    fn collection(mocks: Vec<MockConfig>) -> MockCollectionConfig {
        MockCollectionConfig {
            name: Some("test".to_string()),
            description: None,
            enabled: true,
            vars: None,
            mocks,
            world: None,
            machines: None,
        }
    }

    fn profile() -> Arc<dyn ConsolidationProfile> {
        Arc::new(DefaultProfile)
    }

    fn interaction(uri: &str) -> RecordedInteraction {
        RecordedInteraction {
            id: uri.to_string(),
            timestamp: chrono::Utc::now(),
            request: ferrimock::recorder::RecordedRequest {
                method: "GET".to_string(),
                uri: uri.to_string(),
                query: None,
                headers: Vec::new(),
                body: None,
            },
            response: ferrimock::recorder::RecordedResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: r#"{"id":"1"}"#.to_string(),
            },
            duration: std::time::Duration::from_millis(5),
        }
    }

    #[test]
    fn the_layout_width_matches_the_names() {
        assert_eq!(MERGE_FEATURE_NAMES.len(), MERGE_FEATURE_COUNT);

        let group = [
            mock("a", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("b", "/v2/files/2", 200, r#"{"id":"2"}"#),
        ];
        let profile = DefaultProfile;
        let candidate = MergeCandidate::new(&group, &profile);
        assert_eq!(features_of(&candidate).len(), MERGE_FEATURE_COUNT);
    }

    #[test]
    fn every_dimension_stays_inside_the_unit_interval() {
        // Nothing downstream rescales these, so a dimension that can exceed 1.0
        // would quietly dominate a linear model.
        let big: Vec<MockConfig> = (0..200)
            .map(|n| {
                mock(
                    &format!("m{n}"),
                    &format!("/v2/files/{n}/versions/{n}/content?offset={n}"),
                    200,
                    r#"{"id":"1","total_count":5}"#,
                )
            })
            .collect();
        let profile = DefaultProfile;
        let candidate = MergeCandidate::new(&big, &profile);

        for (name, value) in MERGE_FEATURE_NAMES.iter().zip(features_of(&candidate)) {
            assert!(
                (0.0..=1.0).contains(&value),
                "{name} was {value}, outside [0, 1]"
            );
        }
    }

    #[test]
    fn the_size_threshold_dimension_reports_the_rule_it_competes_with() {
        let profile = DefaultProfile;

        let two = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
        ];
        let candidate = MergeCandidate::new(&two, &profile);
        assert_eq!(features_of(&candidate).last().copied(), Some(0.0));

        let three = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
            mock("c", "/v2/files/3", 200, "{}"),
        ];
        let candidate = MergeCandidate::new(&three, &profile);
        assert_eq!(features_of(&candidate).last().copied(), Some(1.0));
    }

    #[test]
    fn candidate_groups_are_the_ones_the_engine_would_ask_about() {
        let collection = collection(vec![
            mock("f1", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("f2", "/v2/files/2", 200, r#"{"id":"2"}"#),
            mock("f3", "/v2/files/3", 200, r#"{"id":"3"}"#),
            // A different resource: its own group.
            mock("u1", "/v2/users/1", 200, r#"{"id":"1"}"#),
            mock("u2", "/v2/users/2", 200, r#"{"id":"2"}"#),
            // Alone in its group, so never a merge candidate.
            mock("s1", "/v2/status", 200, r#"{"ok":true}"#),
        ]);

        let groups = candidate_groups(&collection, &profile());
        let sizes: Vec<usize> = groups.iter().map(Vec::len).collect();

        assert_eq!(
            sizes,
            vec![3, 2],
            "the two multi-member groups are candidates and the lone mock is not"
        );
    }

    #[test]
    fn a_group_answering_two_different_ways_is_split_before_it_is_offered() {
        let collection = collection(vec![
            mock("f1", "/v2/files/1", 200, r#"{"id":"1"}"#),
            mock("f2", "/v2/files/2", 200, r#"{"id":"2"}"#),
            mock("f3", "/v2/files/3", 404, r#"{"error":"gone"}"#),
            mock("f4", "/v2/files/4", 404, r#"{"error":"gone"}"#),
        ]);

        let groups = candidate_groups(&collection, &profile());
        assert_eq!(
            groups.len(),
            2,
            "the 200s and the 404s are separate decisions, not one"
        );
        for group in &groups {
            let statuses: FxHashSet<Option<u16>> = group
                .iter()
                .map(|mock| {
                    mock.response_config
                        .as_ref()
                        .and_then(ferrimock::config::ResponseConfig::status)
                })
                .collect();
            assert_eq!(statuses.len(), 1, "a partition answers exactly one way");
        }
    }

    #[test]
    fn duplicate_recordings_are_never_offered_as_a_merge_decision() {
        // Identical recordings collapse by a path that never consults a scorer,
        // so labelling them would describe a decision nobody makes.
        let collection = collection(vec![
            mock("d1", "/v2/status", 200, r#"{"ok":true}"#),
            mock("d2", "/v2/status", 200, r#"{"ok":true}"#),
            mock("d3", "/v2/status", 200, r#"{"ok":true}"#),
        ]);

        assert!(candidate_groups(&collection, &profile()).is_empty());
    }

    #[test]
    fn a_labelled_set_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("merges.jsonl");
        let path = path.to_str().unwrap();

        let examples = vec![
            MergeExample {
                group_ids: vec!["a".to_string(), "b".to_string()],
                requests: vec!["GET /a".to_string(), "GET /b".to_string()],
                layout_version: MERGE_FEATURE_LAYOUT_VERSION,
                features: vec![0.5; MERGE_FEATURE_COUNT],
                safe: true,
                behavioral_delta: 0.0,
                value_delta: 0.0,
            },
            MergeExample {
                group_ids: vec!["c".to_string()],
                requests: vec!["GET /c".to_string()],
                layout_version: MERGE_FEATURE_LAYOUT_VERSION,
                features: vec![0.25; MERGE_FEATURE_COUNT],
                safe: false,
                behavioral_delta: 0.0,
                value_delta: -0.5,
            },
        ];

        MergeExample::save(&examples, path).unwrap();
        let loaded = MergeExample::load(path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].safe);
        assert!(!loaded[1].safe);
        assert_eq!(
            loaded[1].value_delta, -0.5,
            "the value level is what separates a good merge from a bad one"
        );
    }

    /// A row whose safety is decided by one dimension the size rule cannot see.
    fn row(id: &str, entropy: f64, size_meets_threshold: bool, safe: bool) -> MergeExample {
        let mut features = vec![0.0; MERGE_FEATURE_COUNT];
        if let Some(slot) = features.get_mut(4) {
            *slot = entropy;
        }
        if let Some(slot) = features.get_mut(MERGE_FEATURE_COUNT - 1) {
            *slot = f64::from(size_meets_threshold);
        }
        MergeExample {
            group_ids: vec![id.to_string()],
            requests: vec![format!("GET /{id}")],
            layout_version: MERGE_FEATURE_LAYOUT_VERSION,
            features,
            safe,
            behavioral_delta: 0.0,
            value_delta: if safe { 0.0 } else { -0.25 },
        }
    }

    #[test]
    fn a_model_fitted_on_a_separable_set_learns_the_dimension_that_decides_it() {
        // Safety tracks path entropy: a position that varies freely is an id and
        // merges cleanly, one that barely varies names a different resource.
        let examples: Vec<MergeExample> = (0..60)
            .map(|n| {
                let high = n % 2 == 0;
                row(
                    &format!("g{n}"),
                    if high { 0.95 } else { 0.1 },
                    n % 3 != 0,
                    high,
                )
            })
            .collect();

        let model = MergeModel::train(&examples, &MergeTrainingConfig::default()).unwrap();

        let safe = model.probability(&row("x", 0.95, false, true).features);
        let risky = model.probability(&row("y", 0.1, true, false).features);
        assert!(
            safe > 0.5 && risky < 0.5,
            "entropy should decide, not size: safe {safe:.3}, risky {risky:.3}"
        );

        let (top, weight) = model.explain().first().copied().unwrap();
        assert_eq!(top, "segment_entropy");
        assert!(weight > 0.0, "higher entropy should argue for merging");
    }

    #[test]
    fn a_model_from_another_feature_layout_declines_rather_than_guesses() {
        let mut model = MergeModel::train(&[], &MergeTrainingConfig::default()).unwrap();
        model.feature_layout_version = MERGE_FEATURE_LAYOUT_VERSION + 1;

        let group = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
        ];
        let profile = DefaultProfile;
        let scorer = LearnedMergeScorer::new(model);

        assert!(
            scorer
                .safe_to_merge(&MergeCandidate::new(&group, &profile))
                .is_none(),
            "declining hands the group back to the size rule; guessing reinterprets every \
             dimension"
        );
    }

    #[test]
    fn rows_from_another_layout_are_refused_rather_than_mixed_in() {
        let mut stale = row("old", 0.5, true, true);
        stale.layout_version = MERGE_FEATURE_LAYOUT_VERSION + 1;

        let error = MergeModel::train(&[stale], &MergeTrainingConfig::default()).unwrap_err();
        assert!(error.contains("feature layout"), "got: {error}");
    }

    #[test]
    fn the_two_mistakes_are_counted_apart() {
        let examples = vec![
            row("a", 0.9, true, true),   // merged, safe
            row("b", 0.1, true, false),  // merged, unsafe -- a broken mock
            row("c", 0.9, false, true),  // refused, safe -- reduction lost
            row("d", 0.1, false, false), // refused, unsafe -- correct
        ];

        let outcome = outcome_of(&examples, size_threshold_merges);
        assert_eq!(
            outcome,
            MergeOutcome {
                merged_safely: 1,
                merged_unsafely: 1,
                refused_safely: 1,
                refused_unsafely: 1,
            }
        );
        assert_eq!(outcome.total(), 4);
        assert_eq!(outcome.unsafe_caught(), 0.5);
        assert_eq!(outcome.safe_merged(), 0.5);
    }

    #[test]
    fn a_rule_that_merges_everything_catches_nothing_unsafe() {
        let examples = vec![row("a", 0.9, true, true), row("b", 0.1, false, false)];
        let outcome = outcome_of(&examples, |_| true);

        assert_eq!(outcome.unsafe_caught(), 0.0);
        assert_eq!(outcome.safe_merged(), 1.0);
    }

    #[test]
    fn a_split_holds_back_the_share_it_was_asked_for_and_keeps_everything() {
        let interactions: Vec<RecordedInteraction> = (0..20)
            .map(|n| interaction(&format!("/v2/files/{n}")))
            .collect();

        let options = MergeLabelOptions {
            holdout_ratio: 0.3,
            seed: 7,
        };
        let (build, holdout) = split_interactions(&interactions, &options);

        assert_eq!(holdout.len(), 6);
        assert_eq!(build.len(), 14);

        let mut seen: Vec<String> = build
            .iter()
            .chain(holdout.iter())
            .map(|interaction| interaction.request.uri.clone())
            .collect();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            20,
            "the split must not drop or duplicate traffic"
        );
    }

    #[test]
    fn a_split_is_reproducible_from_its_seed_and_moves_with_it() {
        let interactions: Vec<RecordedInteraction> = (0..20)
            .map(|n| interaction(&format!("/v2/files/{n}")))
            .collect();

        let uris = |options: &MergeLabelOptions| -> Vec<String> {
            split_interactions(&interactions, options)
                .1
                .iter()
                .map(|interaction| interaction.request.uri.clone())
                .collect()
        };

        let seven = MergeLabelOptions {
            holdout_ratio: 0.3,
            seed: 7,
        };
        let eight = MergeLabelOptions {
            holdout_ratio: 0.3,
            seed: 8,
        };

        assert_eq!(uris(&seven), uris(&seven));
        assert_ne!(
            uris(&seven),
            uris(&eight),
            "a different seed must hold different traffic back, or repeated runs measure one \
             split over and over"
        );
    }

    #[test]
    fn holding_nothing_back_is_refused_rather_than_measured() {
        let interactions = vec![interaction("/v2/files/1")];
        let options = MergeLabelOptions {
            holdout_ratio: 0.0,
            seed: 0,
        };
        let fidelity = FidelityOptions::default();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(label_groups(&interactions, &profile(), &fidelity, &options))
            .unwrap_err();

        assert!(error.contains("unseen traffic"), "got: {error}");
    }

    #[test]
    fn merge_only_answers_for_exactly_one_group() {
        let target = [
            mock("a", "/v2/files/1", 200, "{}"),
            mock("b", "/v2/files/2", 200, "{}"),
        ];
        let other = [
            mock("c", "/v2/users/1", 200, "{}"),
            mock("d", "/v2/users/2", 200, "{}"),
        ];
        let scorer = MergeOnly {
            target: ["a".to_string(), "b".to_string()].into_iter().collect(),
        };
        let profile = DefaultProfile;

        assert_eq!(
            scorer.safe_to_merge(&MergeCandidate::new(&target, &profile)),
            Some(1.0)
        );
        assert_eq!(
            scorer.safe_to_merge(&MergeCandidate::new(&other, &profile)),
            Some(0.0),
            "refusing, not declining: a declined group would merge by the size rule"
        );
    }
}
