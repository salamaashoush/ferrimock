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

pub mod artifact;
pub mod corpus;
pub mod eval;
pub mod extract;
pub mod features;
pub mod generator;
pub mod label;
pub mod linear;
pub mod merge;
pub mod profile;

pub use artifact::ModelArtifact;
pub use corpus::{Corpus, Example, Split};
pub use eval::{Evaluation, evaluate};
pub use features::{FEATURE_COUNT, FEATURE_LAYOUT_VERSION};
pub use label::FieldLabel;
pub use linear::LinearClassifier;
pub use merge::{MERGE_FEATURE_COUNT, MERGE_FEATURE_LAYOUT_VERSION, MergeExample};
pub use profile::LearnedProfile;

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
    fn classify(&self, field_name: &str, values: &[&str]) -> Option<(FieldLabel, f64)>;
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

    fn classify(&self, field_name: &str, values: &[&str]) -> Option<(FieldLabel, f64)> {
        let json: Vec<serde_json::Value> = values
            .iter()
            .map(|value| {
                // A recorded value arrives as text. Numbers and booleans have to
                // go back to their JSON kinds or the detector never sees them as
                // anything but strings.
                serde_json::from_str(value)
                    .ok()
                    .filter(|parsed: &serde_json::Value| {
                        parsed.is_number() || parsed.is_boolean()
                    })
                    .unwrap_or_else(|| serde_json::Value::String((*value).to_string()))
            })
            .collect();
        let refs: Vec<&serde_json::Value> = json.iter().collect();

        let (field_type, confidence) = self.detector.detect_type(field_name, &refs);
        FieldLabel::from_field_type(&field_type).map(|label| (label, confidence))
    }
}
