//! A neural classifier, and the bar it has to clear.
//!
//! The point of this is not that a network is better. It is that the question
//! "is a network better" was never answerable before: the previous attempt had
//! no baseline it could lose to, so it could not find out. Here it is scored by
//! the same [`crate::eval::evaluate`] against the same held-out split as the
//! built-in detector and the linear model on identical features, and
//! [`crate::eval::ShipGate`] refuses it on the same terms.
//!
//! A network on hand-built features usually learns very little the features did
//! not already say. Losing to `LinearClassifier` is the expected outcome and a
//! useful one -- it says the features are the ceiling, and effort belongs in
//! reading raw values rather than in more layers.

use crate::corpus::Corpus;
use crate::features::{FEATURE_COUNT, FEATURE_LAYOUT_VERSION};
use crate::label::FieldLabel;
use crate::Classifier;

use burn::backend::Autodiff;
use burn::backend::ndarray::{NdArray, NdArrayDevice};
use burn::module::{AutodiffModule, Module};
use burn::nn::loss::CrossEntropyLossConfig;
use burn::nn::{Linear, LinearConfig, Relu};
use burn::optim::{AdamConfig, GradientsParams, Optimizer};
use burn::tensor::backend::{AutodiffBackend, Backend};
use burn::tensor::{Int, Tensor, TensorData};

/// CPU backend. Training here is seconds on a corpus this size, and a mock
/// server has no business pulling in a GPU runtime to type a field.
pub type Cpu = NdArray<f32>;
/// The same backend with gradients, for fitting.
pub type Training = Autodiff<Cpu>;

/// Knobs for fitting the network.
#[derive(Debug, Clone)]
pub struct NeuralConfig {
    pub hidden: usize,
    pub epochs: usize,
    pub learning_rate: f64,
    pub batch: usize,
    pub seed: u64,
}

impl Default for NeuralConfig {
    fn default() -> Self {
        Self {
            hidden: 128,
            epochs: 60,
            learning_rate: 1e-3,
            batch: 64,
            seed: 0,
        }
    }
}

/// Two dense layers over the same features the linear model sees.
#[derive(Module, Debug)]
pub struct Mlp<B: Backend> {
    input: Linear<B>,
    hidden: Linear<B>,
    output: Linear<B>,
    activation: Relu,
}

impl<B: Backend> Mlp<B> {
    pub fn new(device: &B::Device, hidden: usize) -> Self {
        Self {
            input: LinearConfig::new(FEATURE_COUNT, hidden).init(device),
            hidden: LinearConfig::new(hidden, hidden).init(device),
            output: LinearConfig::new(hidden, FieldLabel::ALL.len()).init(device),
            activation: Relu::new(),
        }
    }

    pub fn forward(&self, features: Tensor<B, 2>) -> Tensor<B, 2> {
        let x = self.activation.forward(self.input.forward(features));
        let x = self.activation.forward(self.hidden.forward(x));
        self.output.forward(x)
    }
}

/// A fitted network, ready to be scored like any other classifier.
pub struct NeuralClassifier {
    model: Mlp<Cpu>,
    device: NdArrayDevice,
    /// Layout the weights were fitted against. A model read against a different
    /// one would reinterpret every dimension and keep answering confidently.
    pub feature_layout_version: u32,
}

impl NeuralClassifier {
    /// Fit on a corpus.
    pub fn train(corpus: &Corpus, config: &NeuralConfig) -> Self {
        let device = NdArrayDevice::default();
        let mut model: Mlp<Training> = Mlp::new(&device, config.hidden);

        let rows: Vec<(Vec<f32>, usize)> = corpus
            .examples
            .iter()
            .map(|example| (example.features(), example.label.class_index()))
            .collect();

        if rows.is_empty() {
            return Self {
                model: Mlp::new(&device, config.hidden),
                device,
                feature_layout_version: FEATURE_LAYOUT_VERSION,
            };
        }

        let mut optimizer = AdamConfig::new().init();
        let mut order: Vec<usize> = (0..rows.len()).collect();

        for epoch in 0..config.epochs {
            shuffle(&mut order, config.seed ^ epoch as u64);

            for chunk in order.chunks(config.batch.max(1)) {
                let mut features = Vec::with_capacity(chunk.len() * FEATURE_COUNT);
                let mut targets = Vec::with_capacity(chunk.len());
                for index in chunk {
                    let Some((row, label)) = rows.get(*index) else {
                        continue;
                    };
                    features.extend_from_slice(row);
                    targets.push(*label as i32);
                }
                if targets.is_empty() {
                    continue;
                }

                let inputs: Tensor<Training, 2> = Tensor::from_data(
                    TensorData::new(features, [targets.len(), FEATURE_COUNT]),
                    &device,
                );
                let expected: Tensor<Training, 1, Int> =
                    Tensor::from_data(TensorData::new(targets.clone(), [targets.len()]), &device);

                let logits = model.forward(inputs);
                let loss = CrossEntropyLossConfig::new()
                    .init(&device)
                    .forward(logits, expected);

                let gradients = GradientsParams::from_grads(loss.backward(), &model);
                model = optimizer.step(config.learning_rate, model, gradients);
            }
        }

        Self {
            model: model.valid(),
            device,
            feature_layout_version: FEATURE_LAYOUT_VERSION,
        }
    }

    /// Class probabilities for one feature vector.
    fn probabilities(&self, features: &[f32]) -> Vec<f32> {
        let inputs: Tensor<Cpu, 2> = Tensor::from_data(
            TensorData::new(features.to_vec(), [1, FEATURE_COUNT]),
            &self.device,
        );
        let logits = self.model.forward(inputs);
        let probabilities = burn::tensor::activation::softmax(logits, 1);
        probabilities
            .into_data()
            .into_vec()
            .unwrap_or_else(|_| vec![0.0; FieldLabel::ALL.len()])
    }
}

impl Classifier for NeuralClassifier {
    fn name(&self) -> &str {
        "neural"
    }

    fn classify(&self, field_name: &str, values: &[&str]) -> Option<(FieldLabel, f64)> {
        // A model fitted under another layout would read every dimension as
        // something else, so it declines rather than guesses.
        if self.feature_layout_version != FEATURE_LAYOUT_VERSION {
            return None;
        }

        let features = crate::features::extract(field_name, values);
        let probabilities = self.probabilities(&features);

        let (index, confidence) = probabilities
            .iter()
            .enumerate()
            .max_by(|left, right| {
                left.1
                    .partial_cmp(right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, confidence)| (index, f64::from(*confidence)))?;

        FieldLabel::from_class_index(index).map(|label| (label, confidence))
    }
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::corpus::{Example, Provenance};

    fn separable() -> Corpus {
        let mut examples = Vec::new();
        for n in 0..60 {
            examples.push(Example::new(
                "email",
                vec![format!("person{n}@example.com")],
                FieldLabel::Email,
                Provenance::Generated,
            ));
            examples.push(Example::new(
                "id",
                vec![format!("{}-4{}-8{}-{}", n, n, n, n)],
                FieldLabel::Uuid,
                Provenance::Generated,
            ));
        }
        Corpus::new(examples)
    }

    #[test]
    fn it_learns_a_separable_corpus() {
        let corpus = separable();
        let model = NeuralClassifier::train(
            &corpus,
            &NeuralConfig {
                epochs: 25,
                ..NeuralConfig::default()
            },
        );

        let (label, confidence) = model.classify("email", &["someone@example.com"]).unwrap();
        assert_eq!(label, FieldLabel::Email);
        assert!(confidence > 0.5, "confidence was {confidence}");
    }

    #[test]
    fn probabilities_are_a_distribution() {
        let model = NeuralClassifier::train(&separable(), &NeuralConfig::default());
        let features = crate::features::extract("email", &["a@b.com"]);
        let total: f32 = model.probabilities(&features).iter().sum();
        assert!((total - 1.0).abs() < 1e-3, "summed to {total}");
    }

    #[test]
    fn an_untrained_model_still_answers_a_distribution() {
        let model = NeuralClassifier::train(&Corpus::new(Vec::new()), &NeuralConfig::default());
        let answer = model.classify("email", &["a@b.com"]);
        assert!(answer.is_some(), "an empty fit must not panic");
    }

    #[test]
    fn a_model_from_another_feature_layout_declines_rather_than_guesses() {
        let mut model = NeuralClassifier::train(&separable(), &NeuralConfig::default());
        model.feature_layout_version = FEATURE_LAYOUT_VERSION + 1;
        assert!(model.classify("email", &["a@b.com"]).is_none());
    }
}
