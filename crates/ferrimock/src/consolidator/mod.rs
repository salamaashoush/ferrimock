//! Smart consolidation engine for recorded mocks
//!
//! This module provides intelligent consolidation of recorded mock interactions
//! to dramatically reduce file size while maintaining behavioral accuracy.

pub mod analysis;
pub mod fidelity;
pub mod merge;
pub mod pattern;
pub mod provenance;
pub mod shape;

pub use crate::profile::{
    CompositeProfile, ConsolidationProfile, DefaultProfile, PaginationDialect, Placeholder,
    SegmentContext,
};
pub use fidelity::{FidelityOptions, FidelityReport, FidelityScore};
pub use merge::{MergeCandidate, MergeScorer, SizeThreshold};
pub use provenance::Provenance;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_collect
)]
mod tests;

use crate::Result;
use crate::codegen::TemplateGenerator;
use crate::config::{GraphQLMatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use crate::error::Context;
use crate::recorder::RecordedInteraction;
use analysis::{ResponseAnalysis, ResponseAnalyzer};
use pattern::PatternDetector;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

/// Consolidation statistics
#[derive(Debug, Clone)]
pub struct ConsolidationStats {
    pub original_count: usize,
    pub consolidated_count: usize,
    pub reduction_ratio: f64,
    pub patterns_detected: usize,
    pub duplicates_removed: usize,
    pub templates_created: usize,
}

/// Consolidator configuration options
#[derive(Clone)]
pub struct ConsolidatorOptions {
    /// Enable pattern consolidation
    pub enable_consolidation: bool,
    /// Enable template extraction for variable responses
    pub enable_templates: bool,
    /// Minimum number of similar requests to form a pattern
    pub min_pattern_threshold: usize,
    /// Read a value seen once as evidence of what it is, rather than as a
    /// constant.
    ///
    /// Off, a lone recording is reproduced exactly: a value agrees with itself,
    /// so every field reads as fixed and the mock answers the one request it was
    /// recorded at. On, each value is asked what it is, the path is widened
    /// where a segment reads as an identifier, and the result is a template that
    /// answers the whole family of requests -- at the cost of no longer
    /// reproducing the recording verbatim.
    pub generalize: bool,
    /// Enable stateful pagination using persistent storage
    pub enable_stateful_pagination: bool,
    /// Template for storage key pattern (e.g., "api.{path}.total")
    pub pagination_storage_key_template: String,
    /// Domain knowledge consulted ahead of the built-in heuristics.
    ///
    /// Defaults to [`crate::profile::DefaultProfile`], which declines every
    /// domain question and leaves the built-ins in charge.
    pub profile: Arc<dyn ConsolidationProfile>,
    /// Consulted ahead of `min_pattern_threshold` to decide whether a group
    /// merges.
    ///
    /// Defaults to [`merge::SizeThreshold`], which is `min_pattern_threshold`
    /// itself; a scorer that declines leaves that rule in charge.
    pub merge_scorer: Option<Arc<dyn MergeScorer>>,
    /// How sure a scorer has to be before a group is merged.
    pub merge_confidence: f64,
}

impl std::fmt::Debug for ConsolidatorOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsolidatorOptions")
            .field("enable_consolidation", &self.enable_consolidation)
            .field("enable_templates", &self.enable_templates)
            .field("min_pattern_threshold", &self.min_pattern_threshold)
            .field("generalize", &self.generalize)
            .field(
                "enable_stateful_pagination",
                &self.enable_stateful_pagination,
            )
            .field(
                "pagination_storage_key_template",
                &self.pagination_storage_key_template,
            )
            .field("profile", &self.profile.name())
            .field(
                "merge_scorer",
                &self.merge_scorer.as_ref().map(|scorer| scorer.name()),
            )
            .field("merge_confidence", &self.merge_confidence)
            .finish()
    }
}

impl Default for ConsolidatorOptions {
    fn default() -> Self {
        Self {
            enable_consolidation: true,
            enable_templates: true,
            min_pattern_threshold: 3,
            generalize: false,
            enable_stateful_pagination: true,
            pagination_storage_key_template: "api.{path}.total".to_string(),
            profile: crate::profile::default_profile(),
            merge_scorer: None,
            merge_confidence: 0.5,
        }
    }
}

/// Main consolidation engine
pub struct MockConsolidator {
    options: ConsolidatorOptions,
    stats: ConsolidationStats,
    provenance: Provenance,
    pattern_detector: PatternDetector,
    response_analyzer: ResponseAnalyzer,
    template_generator: TemplateGenerator,
}

impl MockConsolidator {
    /// Create a new consolidator with default options
    pub fn new() -> Self {
        Self::with_options(ConsolidatorOptions::default())
    }

    /// Create a new consolidator with custom options
    pub fn with_options(options: ConsolidatorOptions) -> Self {
        let generalize = options.generalize;
        let response_analyzer = ResponseAnalyzer::with_profile(
            options.enable_stateful_pagination,
            Arc::clone(&options.profile),
        )
        .generalizing(generalize);
        let pattern_detector =
            PatternDetector::with_profile(Arc::clone(&options.profile)).generalizing(generalize);
        let template_generator =
            TemplateGenerator::new(options.pagination_storage_key_template.clone());

        Self {
            options,
            stats: ConsolidationStats {
                original_count: 0,
                consolidated_count: 0,
                reduction_ratio: 0.0,
                patterns_detected: 0,
                duplicates_removed: 0,
                templates_created: 0,
            },
            provenance: Provenance::new(),
            pattern_detector,
            response_analyzer,
            template_generator,
        }
    }

    /// Consolidate a mock collection from file
    pub async fn consolidate_file(
        &mut self,
        input_path: impl AsRef<Path>,
    ) -> Result<MockCollectionConfig> {
        let path_buf = PathBuf::from(input_path.as_ref());
        let collection = MockCollectionConfig::from_file(path_buf)
            .await
            .context("Failed to load mock collection")?;

        self.consolidate(collection)
    }

    /// Consolidate a mock collection in memory
    #[allow(clippy::cast_precision_loss)] // Mock counts are small enough for f64 to be exact
    pub fn consolidate(
        &mut self,
        collection: MockCollectionConfig,
    ) -> Result<MockCollectionConfig> {
        self.stats.original_count = collection.mocks.len();
        self.provenance = Provenance::new();

        tracing::debug!(
            mocks = self.stats.original_count,
            "analyzing mocks for consolidation"
        );

        // Streaming mocks (ws/sse) have no consolidatable response shape;
        // pass them through untouched.
        let (streaming_mocks, http_mocks): (Vec<_>, Vec<_>) = collection
            .mocks
            .into_iter()
            .partition(|m| m.sse.is_some() || m.ws.is_some());

        let groups = self.pattern_detector.group_similar_mocks(&http_mocks);
        tracing::debug!(groups = groups.len(), "grouped into request patterns");

        let mut consolidated_mocks = Vec::new();
        for (group_id, group) in groups.iter().enumerate() {
            let processed = self.process_mock_group(group_id, group)?;
            consolidated_mocks.extend(processed);
        }
        for mock in &streaming_mocks {
            self.provenance.record_identity(mock.id.clone());
        }
        consolidated_mocks.extend(streaming_mocks);

        self.stats.consolidated_count = consolidated_mocks.len();
        self.stats.reduction_ratio =
            1.0 - (self.stats.consolidated_count as f64 / self.stats.original_count.max(1) as f64);

        let consolidated_name = collection
            .name
            .map(|n| format!("{n} (Consolidated)"))
            .or_else(|| Some("Consolidated Mocks".to_string()));

        Ok(MockCollectionConfig {
            name: consolidated_name,
            description: Some(format!(
                "Consolidated from {} mocks. Reduction: {:.1}%",
                self.stats.original_count,
                self.stats.reduction_ratio * 100.0
            )),
            enabled: collection.enabled,
            vars: None,
            mocks: consolidated_mocks,
        })
    }

    /// Process a group of similar mocks using generic data-driven algorithm
    #[allow(clippy::indexing_slicing)] // `group[0]` guarded by `group.len() == 1` early return above
    fn process_mock_group(
        &mut self,
        group_id: usize,
        group: &[MockConfig],
    ) -> Result<Vec<MockConfig>> {
        // A lone recording is kept as it was unless it is to be generalized, in
        // which case it goes through the same analysis as any other group and
        // comes back as a template.
        if group.len() == 1 && !self.options.generalize {
            self.record_identity_lineage(group);
            return Ok(group.to_vec());
        }

        tracing::debug!(group = group_id, mocks = group.len(), "processing group");

        if PatternDetector::are_duplicates(group) {
            self.stats.duplicates_removed += group.len() - 1;
            self.stats.patterns_detected += 1;
            tracing::debug!(
                group = group_id,
                removed = group.len() - 1,
                "removed duplicate mocks"
            );
            self.record_group_lineage(&group[0].id, group);
            let mut survivor = group[0].clone();
            // The survivor now answers for all of them, so it must not retire
            // after the first.
            survivor.once = false;
            return Ok(vec![survivor]);
        }

        if !self.options.enable_consolidation {
            tracing::debug!(group = group_id, "consolidation disabled, keeping separate");
            self.record_identity_lineage(group);
            return Ok(group.to_vec());
        }

        // Requests that look alike can still have answered very differently --
        // a 404 among the 200s, a payload carrying a field the others lack.
        // Templating those together makes a mock that is wrong for every member,
        // so split on what was answered before deciding what to merge.
        let partitions = shape::partition_by_response(group);
        if partitions.len() > 1 {
            tracing::debug!(
                group = group_id,
                shapes = partitions.len(),
                sizes = ?partitions.iter().map(Vec::len).collect::<Vec<_>>(),
                "split group by response shape"
            );
        }

        let mut consolidated: Vec<MockConfig> = Vec::new();

        // Partitions arrive largest first, so each one is at least as specific as
        // the last. Giving every later partition a strictly higher priority makes
        // that specificity binding: a lone 404 outranks the `{id}` pattern that
        // would otherwise swallow it, and a rarer response shape is never
        // shadowed by the common one it was split out of.
        for partition in &partitions {
            let mut processed = self.process_partition(partition)?;
            if let Some(floor) = consolidated.iter().map(|mock| mock.priority).max() {
                for mock in &mut processed {
                    if mock.priority <= floor {
                        mock.priority = floor.saturating_add(1);
                    }
                }
            }
            consolidated.extend(processed);
        }

        self.sequence_identical_matchers(&mut consolidated);

        Ok(consolidated)
    }

    /// Replay partitions that match on exactly the same thing as the sequence
    /// they were recorded as.
    ///
    /// Priority encodes specificity, and that only means something when the
    /// matchers differ. One endpoint answering differently over time -- a
    /// GraphQL operation, a token endpoint, a list that grew -- produces
    /// partitions carrying an identical matcher, and raising each one above the
    /// last does not make it more specific. It makes every mock below it
    /// unreachable, so the collection answers the whole endpoint with whichever
    /// recording happened to land on top.
    ///
    /// Ties fall back to collection order, so equal priority plus `once` on all
    /// but the last replays them in turn. A mock standing in for several
    /// recordings answers for all of them and cannot retire, so it goes last
    /// and stays.
    fn sequence_identical_matchers(&self, mocks: &mut [MockConfig]) {
        let mut positions: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (index, mock) in mocks.iter().enumerate() {
            positions
                .entry(Self::matcher_signature(mock))
                .or_default()
                .push(index);
        }

        for indices in positions.values().filter(|indices| indices.len() > 1) {
            // The recording already says how a repeated request replays, in the
            // `once` flags the converter wrote. This does not second-guess that
            // -- inventing a sequence where the recording described none would
            // retire a mock that was meant to keep answering. All that is undone
            // here is the shadowing.
            //
            // Forcing a chain here was tried against a real recording and made
            // fidelity worse: lineage fell further than shape rose, because a
            // retired mock sends the next request somewhere the recording never
            // sent it.
            let mut ordered = indices.clone();
            // A mock standing in for several recordings answers for all of them
            // and cannot retire, so it is the one left holding the endpoint.
            ordered.sort_by_key(|index| {
                let stands_for_many = mocks
                    .get(*index)
                    .is_some_and(|mock| self.provenance.origins(&mock.id).len() > 1);
                (stands_for_many, *index)
            });

            let floor = indices
                .iter()
                .filter_map(|index| mocks.get(*index).map(|mock| mock.priority))
                .min()
                .unwrap_or_else(|| MockConfig::default().priority);

            let resequenced: Vec<MockConfig> = ordered
                .iter()
                .filter_map(|index| mocks.get(*index).cloned())
                .map(|mut mock| {
                    mock.priority = floor;
                    mock
                })
                .collect();

            // Collection order breaks the tie, so they have to sit in the order
            // they answer in. Written rather than swapped: swapping in place
            // moves entries the later indices still refer to.
            let mut slots = indices.clone();
            slots.sort_unstable();
            for (slot, mock) in slots.into_iter().zip(resequenced) {
                if let Some(target) = mocks.get_mut(slot) {
                    *target = mock;
                }
            }
        }
    }

    /// What a mock matches on, as a comparable key.
    fn matcher_signature(mock: &MockConfig) -> String {
        let Some(match_config) = mock.match_config.as_ref() else {
            return format!("none:{}", mock.id);
        };

        let mut methods = match_config.methods.clone();
        if let Some(method) = match_config.method.as_ref() {
            methods.push(method.clone());
        }
        methods.sort_unstable();

        let mut urls: Vec<String> = match_config
            .urls
            .iter()
            .chain(match_config.url.as_ref())
            // `exact:/app-api/graphql` and `/app-api/graphql` are the same URL
            // asked for two ways, and they compete for the same requests. Left
            // unnormalised they hash apart, and a templated mock standing for
            // fourteen recordings sits in its own bucket while the two exact
            // leftovers outrank it -- so the template never answers anything.
            //
            // Only an identical string collapses: a pattern with a placeholder
            // in it is a different string from any exact URL, and stays in its
            // own bucket.
            .map(|url| url.strip_prefix("exact:").unwrap_or(url).to_string())
            .collect();
        urls.sort_unstable();

        // The pinned *values* are what tell two recordings of one URL apart --
        // thirteen calls that each pin a different `$.fileIDs[0]` are thirteen
        // distinguishable mocks, not one shadowing twelve. Comparing only the
        // names would collapse them.
        let mut headers: Vec<String> = match_config
            .headers
            .iter()
            .map(|(name, condition)| format!("{name}={condition:?}"))
            .collect();
        headers.sort_unstable();
        let mut query: Vec<String> = match_config
            .query
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        query.sort_unstable();
        let mut body: Vec<String> = match_config
            .body
            .iter()
            .map(|(path, value)| format!("{path}={value}"))
            .collect();
        body.sort_unstable();

        format!(
            "{methods:?}|{urls:?}|{headers:?}|{query:?}|{body:?}|{:?}",
            match_config.graphql
        )
    }

    /// Whether a group becomes one mock or stays as it was recorded.
    ///
    /// A scorer gets the question first and may decline it; the size threshold
    /// answers whatever is left, which is every group when no scorer is set.
    fn should_merge(&self, group: &[MockConfig]) -> bool {
        if let Some(scorer) = self.options.merge_scorer.as_ref() {
            let candidate = merge::MergeCandidate::new(group, self.options.profile.as_ref());
            if let Some(confidence) = scorer.safe_to_merge(&candidate) {
                let merging = confidence >= self.options.merge_confidence;
                tracing::debug!(
                    mocks = group.len(),
                    scorer = scorer.name(),
                    confidence,
                    required = self.options.merge_confidence,
                    merging,
                    "merge scored"
                );
                return merging;
            }
        }

        let merging = self.options.generalize || group.len() >= self.options.min_pattern_threshold;
        if !merging {
            tracing::debug!(
                mocks = group.len(),
                threshold = self.options.min_pattern_threshold,
                "group below pattern threshold, keeping separate"
            );
        }
        merging
    }

    /// Consolidate one set of mocks that all answered the same way.
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: partitions are never empty
    fn process_partition(&mut self, group: &[MockConfig]) -> Result<Vec<MockConfig>> {
        if !self.should_merge(group) {
            self.record_identity_lineage(group);
            return Ok(group.to_vec());
        }

        let url_pattern = self.pattern_detector.generate_smart_url_pattern(group);
        let response_analysis = self
            .response_analyzer
            .analyze_response_patterns(group, &url_pattern)?;

        // Analyze GraphQL variables if this is a GraphQL group
        let graphql_analysis = ResponseAnalyzer::analyze_graphql_variables(group);

        if graphql_analysis.has_variables {
            tracing::debug!(
                varying = ?graphql_analysis.varying_variables,
                constant = graphql_analysis.constant_variables.len(),
                "analyzed GraphQL variables"
            );
        }

        if response_analysis.varying_fields.is_empty() {
            self.stats.patterns_detected += 1;
            tracing::debug!(
                pattern = %url_pattern,
                "identical responses collapse to a single mock"
            );
            let mut consolidated = group[0].clone();
            consolidated.id = format!("{}-consolidated", group[0].id).into();
            // A mock standing in for a whole group cannot keep the first
            // member's one-shot flag: it would answer the first request and
            // leave every other member of its own group unmatched.
            consolidated.once = false;
            self.record_group_lineage(&consolidated.id.clone(), group);
            if let Some(ref mut match_config) = consolidated.match_config {
                match_config.urls = vec![url_pattern];
                match_config.url = None;
            }
            Self::relax_match_to_group(&mut consolidated, group);
            if self.options.generalize {
                Self::pin_lasting_query(&mut consolidated, group);
            }
            Ok(vec![consolidated])
        } else if self.options.enable_templates && response_analysis.is_json {
            self.stats.patterns_detected += 1;
            tracing::debug!(
                pattern = %url_pattern,
                varying_fields = response_analysis.varying_fields.len(),
                "creating smart template"
            );
            self.stats.templates_created += 1;
            Ok(self.create_smart_template_mock(
                group,
                &url_pattern,
                &response_analysis,
                &graphql_analysis,
            ))
        } else {
            tracing::debug!(
                pattern = %url_pattern,
                "keeping mocks separate: non-JSON responses or templates disabled"
            );
            self.record_identity_lineage(group);
            Ok(group.to_vec())
        }
    }

    /// Pin the query parameters that say what was asked for, and drop the ones
    /// that only say when it was asked.
    ///
    /// A recorded URL carries the whole query, cache buster and all. Pinned as
    /// recorded, the mock waits for `_=1786715224166` -- a number the app
    /// regenerates on every load -- and answers nothing, however well its body
    /// is templated. What is worth keeping names a resource or narrows a search;
    /// a timestamp, a nonce and a session id name the moment.
    ///
    /// The parameters that survive move into `query`, which matches a subset, so
    /// the mock stops caring about the ones it dropped instead of demanding
    /// their absence.
    /// Only a lone recording is rewritten. A merged mock's matcher was already
    /// decided by what its members varied over, which is stronger evidence than
    /// anything a parameter's own shape can offer, and widening it further
    /// measured worse: mocks began answering for each other.
    fn pin_lasting_query(consolidated: &mut MockConfig, group: &[MockConfig]) {
        let [only] = group else {
            return;
        };

        let recorded = Self::query_of(only);
        // Nothing to throw away means nothing to loosen. Moving a query out of
        // the URL trades an exact match for a subset one, and a mock that keeps
        // its exact match answers its own recording and no one else's.
        if !recorded
            .iter()
            .any(|(name, value)| pattern::is_volatile_parameter(name, value))
        {
            return;
        }

        let lasting: FxHashMap<String, String> = recorded
            .into_iter()
            .filter(|(name, value)| !pattern::is_volatile_parameter(name, value))
            .collect();

        let Some(match_config) = consolidated.match_config.as_mut() else {
            return;
        };
        match_config.query = lasting;
        // The parameters that survive move into `query`, which matches a subset,
        // so the mock stops caring about the ones it dropped rather than
        // demanding their absence.
        for url in &mut match_config.urls {
            if let Some((path, _)) = url.split_once('?') {
                *url = path.to_string();
            }
        }
        if let Some(url) = match_config.url.as_mut()
            && let Some((path, _)) = url.split_once('?')
        {
            *url = path.to_string();
        }
    }

    /// Every query parameter a recording matched on, from its URL and from the
    /// parameters the converter pinned one by one.
    fn query_of(mock: &MockConfig) -> Vec<(String, String)> {
        let url = pattern::request_url(mock);
        let mut parameters: Vec<(String, String)> = url
            .split_once('?')
            .map(|(_, query)| {
                url::form_urlencoded::parse(query.as_bytes())
                    .map(|(name, value)| (name.into_owned(), value.into_owned()))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(match_config) = mock.match_config.as_ref() {
            parameters.extend(
                match_config
                    .query
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        parameters
    }

    /// Relax a merged mock's request matchers to cover its whole group.
    ///
    /// A consolidated mock is cloned from one member, so it arrives carrying
    /// that member's pins -- the `$.query` body matcher that told three
    /// different searches apart, the `offset` the first page pinned. Keeping
    /// them means the merged mock answers only the recording it was cloned from
    /// and leaves the rest of its own group unmatched. A pin survives only if
    /// every member agreed on it.
    fn relax_match_to_group(consolidated: &mut MockConfig, group: &[MockConfig]) {
        let Some(match_config) = consolidated.match_config.as_mut() else {
            return;
        };

        match_config.query.retain(|name, value| {
            group.iter().all(|mock| {
                mock.match_config
                    .as_ref()
                    .and_then(|m| m.query.get(name))
                    .is_some_and(|other| other == value)
            })
        });

        match_config.body.retain(|name, value| {
            group.iter().all(|mock| {
                mock.match_config
                    .as_ref()
                    .and_then(|m| m.body.get(name))
                    .is_some_and(|other| other == value)
            })
        });

        // A GraphQL variable is pinned for the same reason a URL segment is
        // literal: it identifies the request. One that varies across the group
        // is the group's placeholder, and keeping it pinned leaves a mock that
        // stands for fourteen recordings matching only the one it was built
        // from -- while the other thirteen match nothing at all.
        if let Some(GraphQLMatchConfig::Structured {
            operation,
            variables,
            ..
        }) = match_config.graphql.as_mut()
        {
            variables.retain(|name, value| {
                group.iter().all(|mock| {
                    Self::graphql_variables(mock)
                        .is_some_and(|other| other.get(name) == Some(value))
                })
            });

            // Nothing left to pin: the operation name is the whole matcher, and
            // saying so plainly beats an empty structured form.
            if variables.is_empty()
                && let Some(operation) = operation.clone()
            {
                match_config.graphql = Some(GraphQLMatchConfig::Simple(operation));
            }
        }
    }

    /// The variables a recorded GraphQL mock pins, if it pins any.
    fn graphql_variables(mock: &MockConfig) -> Option<&FxHashMap<String, serde_json::Value>> {
        match mock.match_config.as_ref()?.graphql.as_ref()? {
            GraphQLMatchConfig::Structured { variables, .. } => Some(variables),
            _ => None,
        }
    }

    /// Every mock in the group survives under its own id.
    fn record_identity_lineage(&mut self, group: &[MockConfig]) {
        for mock in group {
            self.provenance.record_identity(mock.id.clone());
        }
    }

    /// One mock now answers for the whole group.
    fn record_group_lineage(&mut self, consolidated_id: &str, group: &[MockConfig]) {
        self.provenance
            .record(consolidated_id, group.iter().map(|mock| mock.id.clone()));
    }

    /// Create a smart template-based mock using Tera templates
    #[allow(clippy::indexing_slicing)] // `group[0]` guarded by callers ensuring non-empty group
    fn create_smart_template_mock(
        &mut self,
        group: &[MockConfig],
        pattern: &str,
        analysis: &ResponseAnalysis,
        graphql_analysis: &analysis::GraphQLVariableAnalysis,
    ) -> Vec<MockConfig> {
        let base_path = PatternDetector::extract_base_path(&group[0]);

        // Convert consolidator types to codegen types
        let response_structure: crate::codegen::ResponseStructure = analysis.into();
        let graphql_info: crate::codegen::GraphQLVariableInfo = graphql_analysis.into();

        let template_body = self.template_generator.generate_tera_template(
            &response_structure,
            &base_path,
            &graphql_info,
        );

        if let Err(e) = crate::template::validate_template(&template_body) {
            tracing::warn!(
                error = %e,
                template = %template_body,
                "generated template does not validate; keeping the group's mocks separate"
            );
            self.record_identity_lineage(group);
            return group.to_vec();
        }

        let mut template_mock = group[0].clone();
        template_mock.id = format!("{}-smart-template", group[0].id).into();
        // See the identical-response branch: a group's stand-in must outlive the
        // first request it answers.
        template_mock.once = false;
        self.record_group_lineage(&template_mock.id.clone(), group);
        if let Some(ref mut match_config) = template_mock.match_config {
            match_config.urls = vec![pattern.to_string()];
            match_config.url = None;
        }
        Self::relax_match_to_group(&mut template_mock, group);
        if self.options.generalize {
            Self::pin_lasting_query(&mut template_mock, group);
        }

        // Extract common headers and status from the group
        let common_status = Self::extract_common_status(group);
        let common_headers = Self::extract_common_headers(group);

        template_mock.response_config = Some(ReturnConfig::Structured {
            status: common_status,
            headers: common_headers,
            body: None,
            template: Some(template_body),
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        });

        tracing::debug!(
            mock = %template_mock.id,
            dynamic_fields = analysis.varying_fields.len(),
            "generated smart template"
        );

        vec![template_mock]
    }

    /// Consolidate a recording and measure what the consolidation cost.
    ///
    /// `interactions` is the ground truth the collection was recorded from.
    /// Every one of them is replayed against the consolidated collection and
    /// against the original, so the report separates "consolidation broke this"
    /// from "this was never replayable".
    pub async fn consolidate_verified(
        &mut self,
        interactions: &[RecordedInteraction],
        original: MockCollectionConfig,
        fidelity_options: &FidelityOptions,
    ) -> Result<(MockCollectionConfig, FidelityReport)> {
        let consolidated = self.consolidate(original.clone())?;
        let report = fidelity::verify(
            interactions,
            &original,
            &consolidated,
            &self.provenance,
            fidelity_options,
        )
        .await?;
        Ok((consolidated, report))
    }

    /// Get consolidation statistics
    pub fn stats(&self) -> &ConsolidationStats {
        &self.stats
    }

    /// Which original mocks each consolidated mock now answers for.
    ///
    /// Empty until [`Self::consolidate`] has run.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// Extract common status code from a group of mocks (if all are the same)
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.is_empty()` returns early
    fn extract_common_status(group: &[MockConfig]) -> Option<u16> {
        if group.is_empty() {
            return None;
        }

        let first_status = group[0]
            .response_config
            .as_ref()
            .and_then(crate::config::ResponseConfig::status);

        // Check if all mocks have the same status
        let all_same = group.iter().all(|mock| {
            mock.response_config
                .as_ref()
                .and_then(crate::config::ResponseConfig::status)
                == first_status
        });

        if all_same { first_status } else { None }
    }

    /// Extract common headers from a group of mocks
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.is_empty()` returns early
    fn extract_common_headers(group: &[MockConfig]) -> FxHashMap<String, String> {
        if group.is_empty() {
            return FxHashMap::default();
        }

        // Get headers from first mock
        let first_headers = group[0]
            .response_config
            .as_ref()
            .and_then(|r| r.headers())
            .cloned()
            .unwrap_or_default();

        // Find headers that are common across all mocks (same key and value)
        let mut common_headers = FxHashMap::default();

        for (key, value) in &first_headers {
            let is_common = group.iter().all(|mock| {
                mock.response_config
                    .as_ref()
                    .and_then(|r| r.headers())
                    .and_then(|h| h.get(key))
                    .is_some_and(|v| v == value)
            });

            if is_common {
                common_headers.insert(key.clone(), value.clone());
            }
        }

        common_headers
    }
}

impl Default for MockConsolidator {
    fn default() -> Self {
        Self::new()
    }
}
