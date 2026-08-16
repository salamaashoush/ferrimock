//! Multinomial logistic regression over the feature vector.
//!
//! This is the baseline, and the reason it exists is adversarial: a neural
//! network trained on hand-built features often learns nothing the features did
//! not already say. Reporting its accuracy alone would credit the model for work
//! the feature engineering did. A linear model on the identical features
//! separates the two, and anything more elaborate has to beat it.
//!
//! It is also useful on its own. It trains in seconds, its weights say in plain
//! terms which features drove a decision, and it produces calibrated-enough
//! probabilities to compare against the detector's confidence.

// A classifier answering `name()` with a literal reads as needlessly bound
// against the `&str` the trait must return for models that name themselves after
// the artifact they were loaded from.
#![allow(clippy::unnecessary_literal_bound)]

use crate::corpus::Corpus;
use crate::features::{self, FEATURE_COUNT, FEATURE_LAYOUT_VERSION};
use crate::label::FieldLabel;
use crate::{Classifier, artifact::ModelArtifact};
use serde::{Deserialize, Serialize};

/// How training is run.
#[derive(Debug, Clone, Copy)]
pub struct TrainingConfig {
    pub epochs: usize,
    pub learning_rate: f32,
    /// L2 penalty. Some regularisation is not optional here: several features
    /// are near-duplicates of each other, and without it their weights drift
    /// apart to no purpose and the model stops being readable.
    pub l2: f32,
    /// Weight each class by the inverse of its frequency, so a rare class is not
    /// simply ignored in favour of the majority.
    pub balance_classes: bool,
    pub seed: u64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            epochs: 200,
            learning_rate: 0.5,
            l2: 1e-4,
            balance_classes: true,
            seed: 0,
        }
    }
}

/// A trained linear classifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearClassifier {
    /// Layout the weights were trained against. Loading against a different one
    /// would silently reinterpret every dimension.
    pub feature_layout_version: u32,
    /// `classes x features`.
    weights: Vec<Vec<f32>>,
    biases: Vec<f32>,
}

impl LinearClassifier {
    /// Fit to a corpus.
    pub fn train(corpus: &Corpus, config: TrainingConfig) -> Self {
        let classes = FieldLabel::ALL.len();
        let mut model = Self {
            feature_layout_version: FEATURE_LAYOUT_VERSION,
            weights: vec![vec![0.0; FEATURE_COUNT]; classes],
            biases: vec![0.0; classes],
        };

        if corpus.is_empty() {
            return model;
        }

        let examples: Vec<(Vec<f32>, usize)> = corpus
            .examples
            .iter()
            .map(|example| (example.features(), example.label.class_index()))
            .collect();

        let weights_per_class = class_weights(corpus, config.balance_classes);
        let mut order: Vec<usize> = (0..examples.len()).collect();

        for epoch in 0..config.epochs {
            shuffle(&mut order, config.seed ^ epoch as u64);
            // Decay: large steps early to find the region, small steps late so
            // the last epochs settle rather than bounce.
            #[allow(clippy::cast_precision_loss)]
            let rate = config.learning_rate / (1.0 + epoch as f32 / 50.0);

            for &index in &order {
                let Some((features, target)) = examples.get(index) else {
                    continue;
                };
                let probabilities = model.probabilities(features);
                let weight = weights_per_class.get(*target).copied().unwrap_or(1.0);

                for class in 0..classes {
                    let expected = f32::from(u8::from(class == *target));
                    let error = (probabilities.get(class).copied().unwrap_or(0.0) - expected)
                        * weight
                        * rate;
                    if error == 0.0 {
                        continue;
                    }
                    let Some(row) = model.weights.get_mut(class) else {
                        continue;
                    };
                    for (w, f) in row.iter_mut().zip(features.iter()) {
                        let decayed = (-config.l2 * rate).mul_add(*w, *w);
                        *w = (-error).mul_add(*f, decayed);
                    }
                    if let Some(bias) = model.biases.get_mut(class) {
                        *bias -= error;
                    }
                }
            }
        }

        model
    }

    /// Class probabilities for a feature vector.
    fn probabilities(&self, features: &[f32]) -> Vec<f32> {
        let mut logits: Vec<f32> = self
            .weights
            .iter()
            .zip(self.biases.iter())
            .map(|(row, bias)| {
                bias + row
                    .iter()
                    .zip(features.iter())
                    .map(|(w, f)| w * f)
                    .sum::<f32>()
            })
            .collect();

        // Subtract the max before exponentiating: without it a confident model
        // overflows to inf and every probability comes back NaN.
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut total = 0.0;
        for logit in &mut logits {
            *logit = (*logit - max).exp();
            total += *logit;
        }
        if total > 0.0 {
            for logit in &mut logits {
                *logit /= total;
            }
        }
        logits
    }

    /// The features that most drove a class, largest weight first.
    ///
    /// The reason to prefer a linear model when it is good enough: this is a
    /// real answer, not an attribution method that might be wrong.
    pub fn explain(&self, label: FieldLabel, limit: usize) -> Vec<(String, f32)> {
        let names = features::feature_names();
        let Some(row) = self.weights.get(label.class_index()) else {
            return Vec::new();
        };
        let mut weighted: Vec<(String, f32)> = names
            .into_iter()
            .zip(row.iter().copied())
            .filter(|(_, weight)| weight.abs() > 1e-4)
            .collect();
        weighted.sort_by(|a, b| {
            b.1.abs()
                .partial_cmp(&a.1.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        weighted.truncate(limit);
        weighted
    }

    pub fn to_artifact(&self) -> ModelArtifact {
        ModelArtifact::new(self.clone())
    }
}

impl Classifier for LinearClassifier {
    fn name(&self) -> &str {
        "linear"
    }

    fn classify(&self, field: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
        let vector = features::extract(field);
        let probabilities = self.probabilities(&vector);

        let (index, confidence) = probabilities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;

        FieldLabel::from_class_index(index).map(|label| (label, f64::from(*confidence)))
    }
}

/// Per-class multipliers that stop a majority class drowning out the rest.
#[allow(clippy::cast_precision_loss)]
fn class_weights(corpus: &Corpus, balance: bool) -> Vec<f32> {
    let classes = FieldLabel::ALL.len();
    if !balance {
        return vec![1.0; classes];
    }

    let counts = corpus.label_counts();
    let total = corpus.len() as f32;
    (0..classes)
        .map(|index| {
            let count = FieldLabel::from_class_index(index)
                .and_then(|label| counts.get(&label).copied())
                .unwrap_or(0);
            if count == 0 {
                1.0
            } else {
                // Capped: an extremely rare class would otherwise get a weight
                // large enough to destabilise every step it appears in.
                (total / (classes as f32 * count as f32)).clamp(0.1, 10.0)
            }
        })
        .collect()
}

fn shuffle(items: &mut [usize], seed: u64) {
    let mut state = seed | 1;
    let mut next = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for index in (1..items.len()).rev() {
        #[allow(clippy::cast_possible_truncation)] // modulo keeps this below `index`
        let pick = (next() % (index as u64 + 1)) as usize;
        items.swap(index, pick);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::string_slice,
    clippy::cast_precision_loss
)]
mod tests {
    use super::*;
    use crate::corpus::{Example, Provenance};

    fn example(name: &str, values: &[&str], label: FieldLabel) -> Example {
        Example::new(
            name,
            values.iter().map(|v| (*v).to_string()).collect(),
            label,
            Provenance::Generated,
        )
    }

    #[test]
    fn classifying_a_field_is_cheap_enough_to_be_invisible() {
        // The question this answers is whether asking a model costs anything
        // worth avoiding. It runs during consolidation, which is an offline step
        // -- the server answers requests from the collection consolidation wrote,
        // and never loads a model. So the only cost that could matter is the one
        // paid once per field of one recording.
        //
        // A guard rather than a benchmark: if a field ever costs a millisecond,
        // consolidating a large recording stops being interactive, and nobody
        // finds out until they point it at one.
        let model = LinearClassifier::train(&separable_corpus(), TrainingConfig::default());
        let values = [
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "01BX5ZZKBKACTAV9WEVGEMMVRZ",
            "01C0M0Z5F6W3HN9K2QRPX8T4YB",
        ];

        let rounds = 20_000;
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            let _ = model.classify(&crate::Field::new("external_reference", &values));
        }
        let each = started.elapsed() / rounds;

        assert!(
            each < std::time::Duration::from_micros(200),
            "a field cost {each:?}, so a recording of ten thousand fields would cost seconds"
        );
        println!("one field costs {each:?}");
    }

    fn separable_corpus() -> Corpus {
        let mut examples = Vec::new();
        for n in 0..40 {
            examples.push(example(
                "email",
                &[&format!("user{n}@example.com")],
                FieldLabel::Email,
            ));
            examples.push(example(
                "uuid",
                &[&format!("550e8400-e29b-41d4-a716-{n:012x}")],
                FieldLabel::Uuid,
            ));
            examples.push(example("count", &[&format!("{n}")], FieldLabel::Number));
        }
        Corpus::new(examples)
    }

    #[test]
    fn it_learns_a_separable_corpus() {
        let corpus = separable_corpus();
        let model = LinearClassifier::train(&corpus, TrainingConfig::default());
        let evaluation = crate::evaluate(&model, &corpus);

        assert!(
            evaluation.macro_f1() > 0.9,
            "a linear model should separate these easily, got {:.3}\n{}",
            evaluation.macro_f1(),
            evaluation.report()
        );
    }

    #[test]
    fn probabilities_are_a_distribution() {
        let model = LinearClassifier::train(&separable_corpus(), TrainingConfig::default());
        for (name, values) in [
            ("email", vec!["a@b.com"]),
            ("weird", vec![""]),
            ("huge", vec![&"x".repeat(5000)[..]]),
        ] {
            let vector = features::extract(&crate::Field::new(name, &values));
            let probabilities = model.probabilities(&vector);
            let total: f32 = probabilities.iter().sum();

            assert!(
                probabilities.iter().all(|p| p.is_finite() && *p >= 0.0),
                "{name} produced a non-probability"
            );
            assert!(
                (total - 1.0).abs() < 1e-3,
                "{name} probabilities summed to {total}"
            );
        }
    }

    #[test]
    fn an_untrained_model_is_uniform_rather_than_confident() {
        let model = LinearClassifier::train(&Corpus::default(), TrainingConfig::default());
        let (_, confidence) = model
            .classify(&crate::Field::new("anything", &["x"]))
            .unwrap();
        let uniform = 1.0 / FieldLabel::ALL.len() as f64;

        assert!(
            (confidence - uniform).abs() < 1e-6,
            "a model that has seen nothing must not claim to know: {confidence}"
        );
    }

    #[test]
    fn it_says_which_features_drove_a_class() {
        let model = LinearClassifier::train(&separable_corpus(), TrainingConfig::default());
        let explanation = model.explain(FieldLabel::Email, 5);

        assert!(!explanation.is_empty());
        assert!(
            explanation
                .iter()
                .any(|(name, _)| name.contains("email") || name.contains("char.punct")),
            "email should lean on something email-shaped, got {explanation:?}"
        );
    }

    #[test]
    fn balancing_keeps_a_rare_class_alive() {
        let mut examples = Vec::new();
        for n in 0..200 {
            examples.push(example("id", &[&format!("{n}")], FieldLabel::Number));
        }
        for n in 0..8 {
            examples.push(example(
                "email",
                &[&format!("u{n}@example.com")],
                FieldLabel::Email,
            ));
        }
        let corpus = Corpus::new(examples);

        let balanced = LinearClassifier::train(
            &corpus,
            TrainingConfig {
                balance_classes: true,
                ..TrainingConfig::default()
            },
        );
        let unbalanced = LinearClassifier::train(
            &corpus,
            TrainingConfig {
                balance_classes: false,
                ..TrainingConfig::default()
            },
        );

        let recall = |model: &LinearClassifier| {
            crate::evaluate(model, &corpus)
                .per_class
                .get(&FieldLabel::Email)
                .map_or(0.0, ClassScoreExt::recall_of)
        };
        assert!(
            recall(&balanced) >= recall(&unbalanced),
            "balancing made the rare class worse: {:.3} vs {:.3}",
            recall(&balanced),
            recall(&unbalanced)
        );
    }

    trait ClassScoreExt {
        fn recall_of(&self) -> f64;
    }
    impl ClassScoreExt for crate::eval::ClassScore {
        fn recall_of(&self) -> f64 {
            self.recall()
        }
    }
}
