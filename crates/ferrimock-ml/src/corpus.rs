//! Labelled examples, and how they are split.
//!
//! An example is a field name, the values it was seen holding, and what it
//! actually is. That last part is the whole difficulty: a label has to come from
//! something that *knows*, not from something that guesses. Two sources qualify.
//!
//! - A generator: it chose to emit an email, so the example is an email.
//! - A person: they read the recording and said so.
//!
//! The detector is not a source. Labelling with it and then measuring against it
//! measures nothing.

use crate::features;
use crate::label::FieldLabel;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Where an example's label came from. Recorded because it decides what a
/// measurement over the example is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Emitted by a generator that knew what it was making.
    Generated,
    /// Read off real traffic and labelled by a person.
    Reviewed,
}

/// One labelled field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Example {
    pub field_name: String,
    pub values: Vec<String>,
    pub label: FieldLabel,
    pub provenance: Provenance,
}

impl Example {
    pub fn new(
        field_name: impl Into<String>,
        values: Vec<String>,
        label: FieldLabel,
        provenance: Provenance,
    ) -> Self {
        Self {
            field_name: field_name.into(),
            values,
            label,
            provenance,
        }
    }

    pub fn features(&self) -> Vec<f32> {
        let refs: Vec<&str> = self.values.iter().map(String::as_str).collect();
        features::extract(&self.field_name, &refs)
    }

    pub fn value_refs(&self) -> Vec<&str> {
        self.values.iter().map(String::as_str).collect()
    }
}

/// A set of labelled examples.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    pub examples: Vec<Example>,
}

impl Corpus {
    pub fn new(examples: Vec<Example>) -> Self {
        Self { examples }
    }

    pub fn len(&self) -> usize {
        self.examples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.examples.is_empty()
    }

    /// How many examples carry each label.
    pub fn label_counts(&self) -> FxHashMap<FieldLabel, usize> {
        let mut counts = FxHashMap::default();
        for example in &self.examples {
            *counts.entry(example.label).or_insert(0) += 1;
        }
        counts
    }

    /// Only the examples a person vouched for.
    #[must_use]
    pub fn reviewed_only(&self) -> Self {
        Self::new(
            self.examples
                .iter()
                .filter(|e| e.provenance == Provenance::Reviewed)
                .cloned()
                .collect(),
        )
    }

    /// Read a corpus from JSON Lines.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let examples = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<Example>, _>>()
            .map_err(std::io::Error::other)?;
        Ok(Self::new(examples))
    }

    /// Write a corpus as JSON Lines -- one example per line, so a corpus can be
    /// appended to, diffed, and reviewed a line at a time.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let mut text = String::new();
        for example in &self.examples {
            text.push_str(&serde_json::to_string(example).map_err(std::io::Error::other)?);
            text.push('\n');
        }
        std::fs::write(path, text)
    }

    /// Split into train / validation / test, keeping each label's proportions.
    ///
    /// Stratified because the label distribution is nowhere near uniform: a
    /// random split can leave a rare class absent from test entirely, and a
    /// macro-averaged score over classes that are not there is a fiction.
    ///
    /// `seed` makes the split reproducible, so two runs compare like with like.
    pub fn split(&self, train: f64, validation: f64, seed: u64) -> Split {
        let mut by_label: FxHashMap<FieldLabel, Vec<Example>> = FxHashMap::default();
        for example in &self.examples {
            by_label
                .entry(example.label)
                .or_default()
                .push(example.clone());
        }

        let mut split = Split::default();
        // Iterate labels in a fixed order: hash order would make the split
        // depend on the map's internals rather than on the seed.
        let mut labels: Vec<FieldLabel> = by_label.keys().copied().collect();
        labels.sort_unstable();

        for label in labels {
            let Some(mut group) = by_label.remove(&label) else {
                continue;
            };
            shuffle(&mut group, seed ^ (label.class_index() as u64).wrapping_mul(0x9E37));

            let portion = |fraction: f64| -> usize {
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let count = (group.len() as f64 * fraction.max(0.0)).round() as usize;
                count.min(group.len())
            };
            let train_end = portion(train);
            let validation_end = (train_end + portion(validation)).min(group.len());

            for (index, example) in group.into_iter().enumerate() {
                if index < train_end {
                    split.train.examples.push(example);
                } else if index < validation_end {
                    split.validation.examples.push(example);
                } else {
                    split.test.examples.push(example);
                }
            }
        }

        split
    }
}

/// A corpus divided for training and honest measurement.
#[derive(Debug, Clone, Default)]
pub struct Split {
    pub train: Corpus,
    pub validation: Corpus,
    /// Touched once, at the end. Anything tuned against it stops being a
    /// measurement of how the model will do on data it has not seen.
    pub test: Corpus,
}

/// Deterministic shuffle. A named, seeded permutation beats a thread RNG here:
/// every run must produce the same split from the same seed.
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut state = seed | 1;
    let mut next = || {
        // xorshift64*: small, deterministic, and good enough to order a list.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for index in (1..items.len()).rev() {
        #[allow(clippy::cast_possible_truncation)] // Bounded by `index`, a usize already
        let pick = (next() % (index as u64 + 1)) as usize;
        items.swap(index, pick);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn example(label: FieldLabel, n: usize) -> Example {
        Example::new(
            format!("f{n}"),
            vec![format!("v{n}")],
            label,
            Provenance::Generated,
        )
    }

    fn corpus(counts: &[(FieldLabel, usize)]) -> Corpus {
        let mut examples = Vec::new();
        let mut n = 0;
        for (label, count) in counts {
            for _ in 0..*count {
                examples.push(example(*label, n));
                n += 1;
            }
        }
        Corpus::new(examples)
    }

    #[test]
    fn a_split_keeps_every_label_in_every_part() {
        let corpus = corpus(&[
            (FieldLabel::Email, 100),
            (FieldLabel::Uuid, 100),
            (FieldLabel::Opaque, 100),
        ]);
        let split = corpus.split(0.7, 0.15, 42);

        for part in [&split.train, &split.validation, &split.test] {
            let counts = part.label_counts();
            assert_eq!(
                counts.len(),
                3,
                "a label went missing from a split, so any macro average over it is fiction"
            );
        }
        assert_eq!(
            split.train.len() + split.validation.len() + split.test.len(),
            300
        );
    }

    #[test]
    fn a_rare_label_still_reaches_the_test_set() {
        let corpus = corpus(&[(FieldLabel::Email, 200), (FieldLabel::Timezone, 10)]);
        let split = corpus.split(0.7, 0.15, 7);

        assert!(
            split.test.label_counts().contains_key(&FieldLabel::Timezone),
            "stratification exists precisely so the rare class is measured"
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_split() {
        let corpus = corpus(&[(FieldLabel::Email, 50), (FieldLabel::Url, 50)]);
        let first = corpus.split(0.7, 0.15, 99);
        let second = corpus.split(0.7, 0.15, 99);

        let names = |c: &Corpus| -> Vec<String> {
            c.examples.iter().map(|e| e.field_name.clone()).collect()
        };
        assert_eq!(names(&first.train), names(&second.train));
        assert_eq!(names(&first.test), names(&second.test));
    }

    #[test]
    fn a_different_seed_gives_a_different_split() {
        let corpus = corpus(&[(FieldLabel::Email, 100)]);
        let first = corpus.split(0.7, 0.15, 1);
        let second = corpus.split(0.7, 0.15, 2);

        let names = |c: &Corpus| -> Vec<String> {
            c.examples.iter().map(|e| e.field_name.clone()).collect()
        };
        assert_ne!(names(&first.train), names(&second.train));
    }

    #[test]
    fn nothing_appears_in_two_parts_at_once() {
        let corpus = corpus(&[(FieldLabel::Email, 60), (FieldLabel::Uuid, 60)]);
        let split = corpus.split(0.7, 0.15, 3);

        let mut all: Vec<String> = [&split.train, &split.validation, &split.test]
            .iter()
            .flat_map(|part| part.examples.iter().map(|e| e.field_name.clone()))
            .collect();
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "an example leaked across the split");
    }

    #[test]
    fn a_corpus_round_trips_through_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corpus.jsonl");

        let original = corpus(&[(FieldLabel::Email, 3), (FieldLabel::Uuid, 2)]);
        original.save(&path).unwrap();
        let loaded = Corpus::load(&path).unwrap();

        assert_eq!(loaded.len(), original.len());
        assert_eq!(loaded.label_counts(), original.label_counts());
    }

    #[test]
    fn reviewed_examples_can_be_isolated() {
        let mut examples = vec![example(FieldLabel::Email, 1)];
        examples.push(Example::new(
            "real",
            vec!["a@b.com".to_string()],
            FieldLabel::Email,
            Provenance::Reviewed,
        ));
        let corpus = Corpus::new(examples);

        assert_eq!(corpus.len(), 2);
        assert_eq!(corpus.reviewed_only().len(), 1);
    }
}
