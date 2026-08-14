//! Plugging a trained model into consolidation.
//!
//! A model reaches the engine the same way any other domain knowledge does: as a
//! [`ConsolidationProfile`]. It answers `classify_field` and declines everything
//! else, so it composes with a hand-written profile through
//! [`ferrimock::profile::CompositeProfile`] -- hand-written rules first, the
//! model behind them.
//!
//! ## Only the residual
//!
//! The model is not asked about fields the built-in detector already places.
//! Two reasons, and the second is the important one.
//!
//! The detector is a regex away from certain on a UUID, and no model improves on
//! that. But `classify_field` is consulted *before* the detector, so answering
//! everything would mean overriding it everywhere -- including where it is
//! right. Declining unless the detector would have said `RandomString` keeps the
//! model to the ground the detector conceded, which is the ground it was trained
//! to cover.

use crate::{Classifier, label::FieldLabel};
use ferrimock::profile::ConsolidationProfile;
use ferrimock::type_detector::{FieldType, TypeDetector};
use serde_json::Value as JsonValue;

/// A trained classifier, as a profile.
pub struct LearnedProfile<C: Classifier> {
    classifier: C,
    detector: TypeDetector,
    minimum_confidence: f64,
    name: String,
}

impl<C: Classifier> LearnedProfile<C> {
    /// Confidence below which the model stays quiet.
    ///
    /// The alternative to answering is `RandomString`, which is not much of a
    /// bar -- but a wrong specific answer is worse than a vague one. A guess at
    /// `email` makes a template generate addresses for a field that holds
    /// something else entirely.
    pub const DEFAULT_MINIMUM_CONFIDENCE: f64 = 0.6;

    pub fn new(classifier: C) -> Self {
        let name = format!("learned:{}", classifier.name());
        Self {
            classifier,
            detector: TypeDetector::new(),
            minimum_confidence: Self::DEFAULT_MINIMUM_CONFIDENCE,
            name,
        }
    }

    #[must_use]
    pub fn with_minimum_confidence(mut self, minimum: f64) -> Self {
        self.minimum_confidence = minimum;
        self
    }

    /// Whether the built-in detector already has an answer worth keeping.
    fn detector_placed_it(&self, field: &str, values: &[&JsonValue]) -> bool {
        let (field_type, _) = self.detector.detect_type(field, values);
        !matches!(field_type, FieldType::RandomString)
    }
}

impl<C: Classifier + Send + Sync> ConsolidationProfile for LearnedProfile<C> {
    fn name(&self) -> &str {
        &self.name
    }

    fn classify_field(&self, field: &str, values: &[&JsonValue]) -> Option<(FieldType, f64)> {
        if values.is_empty() || self.detector_placed_it(field, values) {
            return None;
        }

        // Strings reach the model unquoted; anything else keeps its JSON form so
        // a number still reads as a number.
        let rendered: Vec<String> = values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), std::string::ToString::to_string)
            })
            .collect();
        let refs: Vec<&str> = rendered.iter().map(String::as_str).collect();

        let (label, confidence) = self.classifier.classify(field, &refs)?;
        if confidence < self.minimum_confidence || label == FieldLabel::Opaque {
            // `Opaque` is what the detector already said. Repeating it back
            // gains nothing and would only overwrite its confidence.
            return None;
        }

        Some((label.to_field_type(), confidence))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct Fixed(FieldLabel, f64);
    impl Classifier for Fixed {
        fn name(&self) -> &str {
            "fixed"
        }
        fn classify(&self, _: &str, _: &[&str]) -> Option<(FieldLabel, f64)> {
            Some((self.0, self.1))
        }
    }

    fn values(raw: &[&str]) -> Vec<JsonValue> {
        raw.iter().map(|v| JsonValue::String((*v).to_string())).collect()
    }

    #[test]
    fn the_model_is_not_asked_about_fields_the_detector_places() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::PhoneNumber, 0.99));
        let owned = values(&["a@b.com", "c@d.org"]);
        let refs: Vec<&JsonValue> = owned.iter().collect();

        assert!(
            profile.classify_field("email", &refs).is_none(),
            "the detector already knows an email; overriding it is how a model \
             makes things worse"
        );
    }

    #[test]
    fn the_model_answers_where_the_detector_gave_up() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::Token, 0.9));
        let owned = values(&["zx8k2", "pq4m9", "tt7b1"]);
        let refs: Vec<&JsonValue> = owned.iter().collect();

        assert_eq!(
            profile.classify_field("blob", &refs),
            Some((FieldType::Token, 0.9))
        );
    }

    #[test]
    fn a_low_confidence_guess_is_withheld() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::Token, 0.2));
        let owned = values(&["zx8k2", "pq4m9"]);
        let refs: Vec<&JsonValue> = owned.iter().collect();

        assert!(profile.classify_field("blob", &refs).is_none());
    }

    #[test]
    fn predicting_opaque_says_nothing_the_detector_had_not_said() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::Opaque, 0.99));
        let owned = values(&["zx8k2", "pq4m9"]);
        let refs: Vec<&JsonValue> = owned.iter().collect();

        assert!(profile.classify_field("blob", &refs).is_none());
    }

    #[test]
    fn a_field_with_no_samples_is_not_guessed_at() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::Email, 0.99));
        assert!(profile.classify_field("anything", &[]).is_none());
    }

    #[test]
    fn the_threshold_is_adjustable() {
        let owned = values(&["zx8k2", "pq4m9"]);
        let refs: Vec<&JsonValue> = owned.iter().collect();

        let strict = LearnedProfile::new(Fixed(FieldLabel::Token, 0.5));
        assert!(strict.classify_field("blob", &refs).is_none());

        let lenient = LearnedProfile::new(Fixed(FieldLabel::Token, 0.5))
            .with_minimum_confidence(0.4);
        assert!(lenient.classify_field("blob", &refs).is_some());
    }

    #[test]
    fn the_profile_declines_everything_that_is_not_its_business() {
        let profile = LearnedProfile::new(Fixed(FieldLabel::Email, 0.99));

        assert!(profile.pagination_dialect().is_none());
        assert!(profile.resource_key("/v2/files/1").is_none());
        assert!(!profile.is_download_url("https://files.example.com/x"));
        assert!(profile.redact("access_token", &JsonValue::Null).is_none());
        assert!(profile.name().starts_with("learned:"));
    }
}
