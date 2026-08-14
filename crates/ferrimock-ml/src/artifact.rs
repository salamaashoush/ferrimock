//! Saving and loading a trained model.
//!
//! An artifact records the feature layout it was trained under and refuses to
//! load against a different one. That guard is the whole reason this module
//! exists as more than `serde_json::to_string`.
//!
//! Without it the failure is silent and total: insert a feature in the middle,
//! retrain nothing, and every dimension after it shifts by one. The weights
//! still multiply, the softmax still sums to one, and the model answers with
//! total confidence about a vector that no longer means what it did.

use crate::features::FEATURE_LAYOUT_VERSION;
use crate::linear::LinearClassifier;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A trained model, on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelArtifact {
    /// Feature layout the weights were fitted against.
    pub feature_layout_version: u32,
    /// Free-form note -- what corpus, what date, who trained it.
    #[serde(default)]
    pub note: String,
    pub model: LinearClassifier,
}

/// Why an artifact could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Malformed(serde_json::Error),
    /// The artifact was trained against a different feature layout. Retraining
    /// is the only fix; there is no safe way to reinterpret the weights.
    LayoutMismatch { artifact: u32, current: u32 },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read model: {error}"),
            Self::Malformed(error) => write!(f, "model file is not a valid artifact: {error}"),
            Self::LayoutMismatch { artifact, current } => write!(
                f,
                "model was trained against feature layout v{artifact} but this build uses \
                 v{current}; the weights no longer line up with the features, so it must be \
                 retrained"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

impl ModelArtifact {
    pub fn new(model: LinearClassifier) -> Self {
        Self {
            feature_layout_version: FEATURE_LAYOUT_VERSION,
            note: String::new(),
            model,
        }
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Load, refusing anything trained against a different feature layout.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let text = std::fs::read_to_string(path).map_err(LoadError::Io)?;
        let artifact: Self = serde_json::from_str(&text).map_err(LoadError::Malformed)?;

        if artifact.feature_layout_version != FEATURE_LAYOUT_VERSION {
            return Err(LoadError::LayoutMismatch {
                artifact: artifact.feature_layout_version,
                current: FEATURE_LAYOUT_VERSION,
            });
        }
        // The model carries the version too, and a file edited by hand could
        // disagree with its own envelope.
        if artifact.model.feature_layout_version != FEATURE_LAYOUT_VERSION {
            return Err(LoadError::LayoutMismatch {
                artifact: artifact.model.feature_layout_version,
                current: FEATURE_LAYOUT_VERSION,
            });
        }

        Ok(artifact)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::Classifier;
    use crate::corpus::{Corpus, Example, Provenance};
    use crate::label::FieldLabel;
    use crate::linear::TrainingConfig;

    fn trained() -> LinearClassifier {
        let examples = (0..20)
            .map(|n| {
                Example::new(
                    "email",
                    vec![format!("u{n}@example.com")],
                    FieldLabel::Email,
                    Provenance::Generated,
                )
            })
            .collect();
        LinearClassifier::train(&Corpus::new(examples), TrainingConfig::default())
    }

    #[test]
    fn a_model_survives_the_round_trip_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");

        let model = trained();
        model.to_artifact().with_note("test").save(&path).unwrap();
        let loaded = ModelArtifact::load(&path).unwrap();

        assert_eq!(loaded.note, "test");
        assert_eq!(
            model.classify("email", &["a@b.com"]),
            loaded.model.classify("email", &["a@b.com"]),
            "a reloaded model must answer identically"
        );
    }

    #[test]
    fn an_artifact_from_another_feature_layout_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stale.json");

        let mut artifact = trained().to_artifact();
        artifact.feature_layout_version = FEATURE_LAYOUT_VERSION + 1;
        artifact.save(&path).unwrap();

        match ModelArtifact::load(&path) {
            Err(LoadError::LayoutMismatch { artifact, current }) => {
                assert_eq!(artifact, FEATURE_LAYOUT_VERSION + 1);
                assert_eq!(current, FEATURE_LAYOUT_VERSION);
            }
            other => panic!("a stale artifact loaded anyway: {other:?}"),
        }
    }

    #[test]
    fn an_envelope_that_disagrees_with_its_own_weights_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tampered.json");

        let mut artifact = trained().to_artifact();
        artifact.model.feature_layout_version = FEATURE_LAYOUT_VERSION + 3;
        artifact.save(&path).unwrap();

        assert!(matches!(
            ModelArtifact::load(&path),
            Err(LoadError::LayoutMismatch { .. })
        ));
    }

    #[test]
    fn the_mismatch_message_says_to_retrain() {
        let error = LoadError::LayoutMismatch {
            artifact: 1,
            current: 2,
        };
        let message = error.to_string();
        assert!(message.contains("retrained"), "got: {message}");
    }

    #[test]
    fn a_missing_file_is_an_io_error_not_a_panic() {
        assert!(matches!(
            ModelArtifact::load("/nonexistent/model.json"),
            Err(LoadError::Io(_))
        ));
    }
}
