//! Learned field-type classification for the consolidation engine.
//!
//! The engine's built-in detector is a large, careful pile of heuristics. It is
//! good at what it covers and silent about what it does not: fields it cannot
//! place come back as `RandomString`, and a template then fills them with
//! arbitrary text. That residual is what a model is for.
//!
//! ## What this is not
//!
//! It is not a replacement for the detector. The detector is consulted first,
//! and a model is asked only about fields it declined to place. A regex that
//! recognises a UUID does not need help.
//!
//! ## The methodology that failed before
//!
//! An earlier attempt trained a network on synthetic data labelled by the
//! detector itself. A student taught by a teacher can at best match the teacher,
//! and it did -- which read as success until you noticed it could never have
//! read as anything else. Here, labels come from the generator that produced the
//! value, or from a human-reviewed corpus. Never from the detector.
//!
//! ## The bar
//!
//! [`eval`] scores any classifier -- the detector, a linear baseline, a trained
//! model -- the same way, on the same held-out split. A model ships only if it
//! beats both the detector and the baseline. `report` prints the comparison so
//! that claim is checkable rather than asserted.

// A classifier answering `name()` with a literal reads as needlessly bound
// against the `&str` the trait must return for models that name themselves after
// the artifact they were loaded from.
#![allow(clippy::unnecessary_literal_bound)]

pub mod artifact;
pub mod audit;
pub mod corpus;
pub mod detector;
pub mod eval;
pub mod extract;
pub mod features;
pub mod generator;
pub mod label;
pub mod linear;
pub mod merge;
pub mod neural;
pub mod profile;
pub mod shape;

pub use artifact::ModelArtifact;
pub use audit::{Audit, Defect};
pub use corpus::{Corpus, Example, Split, ValueKind};
pub use eval::{
    Evaluation, HeldOutScore, SourceScore, evaluate, held_out, held_out_report, per_source,
};
pub use features::{FEATURE_COUNT, FEATURE_LAYOUT_VERSION};
pub use generator::Recipe;
pub use generator::census::Census;
pub use generator::dialect::ApiDialect;
pub use label::FieldLabel;
pub use linear::LinearClassifier;
pub use merge::{MERGE_FEATURE_COUNT, MERGE_FEATURE_LAYOUT_VERSION, MergeExample};
pub use neural::{NeuralClassifier, NeuralConfig};
pub use profile::LearnedProfile;

/// One field, as everything that reads it sees it.
///
/// A struct rather than a pair of arguments because what a field *is* keeps
/// growing: it started as a name and some text, gained the JSON kind those
/// values were recorded as, and will gain where the field sits in the response
/// and what the request asked for. Every one of those is evidence, and adding
/// evidence should not mean changing every signature that carries it.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    pub name: &'a str,
    /// The values as text, however they were recorded.
    pub values: &'a [&'a str],
    /// The JSON kind they were recorded as. A count and a numeric string id are
    /// the same digits and differ only in this.
    pub kind: corpus::ValueKind,
}

impl<'a> Field<'a> {
    /// A field whose values were recorded as JSON strings.
    pub fn new(name: &'a str, values: &'a [&'a str]) -> Self {
        Self {
            name,
            values,
            kind: corpus::ValueKind::String,
        }
    }

    #[must_use]
    pub fn of_kind(mut self, kind: corpus::ValueKind) -> Self {
        self.kind = kind;
        self
    }

    /// The values, back in the JSON kinds they were recorded as.
    pub fn json_values(&self) -> Vec<serde_json::Value> {
        self.values
            .iter()
            .map(|value| self.kind.as_json(value))
            .collect()
    }
}

/// Anything that can name a field's type from its name and sampled values.
///
/// The detector implements this, so does a linear model, so does anything
/// trained later. That is the point: they are scored through one interface, on
/// one split, and a new model has to earn its place against the others.
pub trait Classifier {
    /// A short name for reports.
    fn name(&self) -> &str;

    /// The predicted label and how confident the classifier is in it.
    ///
    /// Confidence must mean the same thing across implementations -- roughly,
    /// the probability the label is right -- because callers compare them.
    fn classify(&self, field: &Field<'_>) -> Option<(FieldLabel, f64)>;
}

/// The engine's built-in detector, wrapped so it can be scored like a model.
pub struct HeuristicClassifier {
    detector: ferrimock::type_detector::TypeDetector,
}

impl Default for HeuristicClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl HeuristicClassifier {
    pub fn new() -> Self {
        Self {
            detector: ferrimock::type_detector::TypeDetector::new(),
        }
    }
}

impl Classifier for HeuristicClassifier {
    fn name(&self) -> &str {
        "heuristic"
    }

    fn classify(&self, field: &Field<'_>) -> Option<(FieldLabel, f64)> {
        let (field_type, confidence) = detector::detect(&self.detector, field);
        FieldLabel::from_field_type(&field_type).map(|label| (label, confidence))
    }
}
