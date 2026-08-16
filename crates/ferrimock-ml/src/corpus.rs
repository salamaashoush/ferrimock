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

/// The JSON kind a field's values were recorded as.
///
/// Not cosmetic, and not derivable from the text. A numeric string id and a
/// count are the same digits and differ in exactly one thing: whether the JSON
/// had quotes around them. A corpus that stores both as text and lets the reader
/// guess has thrown away the only evidence that separates them, and then
/// measures the detector against a question nobody could answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    #[default]
    String,
    Number,
    Boolean,
}

impl ValueKind {
    /// A recorded value, back in the kind it was recorded as.
    pub fn as_json(self, value: &str) -> serde_json::Value {
        match self {
            Self::String => serde_json::Value::String(value.to_string()),
            Self::Number | Self::Boolean => serde_json::from_str(value)
                .ok()
                .filter(|parsed: &serde_json::Value| parsed.is_number() || parsed.is_boolean())
                // A disturbed sample is not a number any more -- a redacted count
                // comes back as `***` -- and it stays the text it became.
                .unwrap_or_else(|| serde_json::Value::String(value.to_string())),
        }
    }
}

/// One labelled field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Example {
    pub field_name: String,
    pub values: Vec<String>,
    pub label: FieldLabel,
    pub provenance: Provenance,
    /// Which API this field came from -- a generated family, or the host a
    /// recording was taken against.
    ///
    /// The only way to ask whether a model works on an API it has never seen is
    /// to hold one out of training and score on it, and that question cannot
    /// even be posed unless every example remembers where it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The JSON kind the values were recorded as.
    #[serde(default, skip_serializing_if = "is_string_kind")]
    pub kind: ValueKind,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's `skip_serializing_if` hands a reference
fn is_string_kind(kind: &ValueKind) -> bool {
    *kind == ValueKind::String
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
            source: None,
            kind: ValueKind::String,
        }
    }

    /// The same example, recorded as the JSON kind it really had.
    #[must_use]
    pub fn of_kind(mut self, kind: ValueKind) -> Self {
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

    /// The same example, attributed to the API it came from.
    #[must_use]
    pub fn from_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn features(&self) -> Vec<f32> {
        let refs: Vec<&str> = self.values.iter().map(String::as_str).collect();
        features::extract(&self.as_field(&refs))
    }

    pub fn value_refs(&self) -> Vec<&str> {
        self.values.iter().map(String::as_str).collect()
    }

    /// The example as a classifier sees it.
    ///
    /// Borrows the values, so the caller holds them: `let values =
    /// example.value_refs(); example.as_field(&values)`.
    pub fn as_field<'a>(&'a self, values: &'a [&'a str]) -> crate::Field<'a> {
        crate::Field {
            name: &self.field_name,
            values,
            kind: self.kind,
        }
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

    /// The APIs this corpus was drawn from, in a fixed order.
    pub fn sources(&self) -> Vec<String> {
        let mut sources: Vec<String> = self
            .examples
            .iter()
            .filter_map(|example| example.source.clone())
            .collect();
        sources.sort();
        sources.dedup();
        sources
    }

    /// Everything except one API.
    ///
    /// The training half of a held-out-API measurement. Examples with no source
    /// at all stay in: they are usually reviewed real traffic, and dropping them
    /// would quietly shrink the only data that matters.
    #[must_use]
    pub fn without_source(&self, source: &str) -> Self {
        Self::new(
            self.examples
                .iter()
                .filter(|example| example.source.as_deref() != Some(source))
                .cloned()
                .collect(),
        )
    }

    /// Only one API. The scoring half of a held-out-API measurement.
    #[must_use]
    pub fn only_source(&self, source: &str) -> Self {
        Self::new(
            self.examples
                .iter()
                .filter(|example| example.source.as_deref() == Some(source))
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
            shuffle(
                &mut group,
                seed ^ (label.class_index() as u64).wrapping_mul(0x9E37),
            );

            let portion = |fraction: f64| -> usize {
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss
                )]
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
            split
                .test
                .label_counts()
                .contains_key(&FieldLabel::Timezone),
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
    fn an_api_can_be_held_out_and_scored_on_separately() {
        let corpus = Corpus::new(vec![
            example(FieldLabel::Email, 1).from_source("payments-platform"),
            example(FieldLabel::Email, 2).from_source("payments-platform"),
            example(FieldLabel::Uuid, 3).from_source("content-platform"),
            // Reviewed traffic often has no family; it must survive the split.
            example(FieldLabel::Uuid, 4),
        ]);

        assert_eq!(
            corpus.sources(),
            vec!["content-platform", "payments-platform"]
        );
        assert_eq!(corpus.only_source("payments-platform").len(), 2);
        assert_eq!(
            corpus.without_source("payments-platform").len(),
            2,
            "the unattributed example belongs to no family and stays in training"
        );
    }

    #[test]
    fn a_source_survives_the_round_trip_and_its_absence_does_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sourced.jsonl");

        let original = Corpus::new(vec![
            example(FieldLabel::Email, 1).from_source("developer-platform"),
            example(FieldLabel::Uuid, 2),
        ]);
        original.save(&path).unwrap();
        let loaded = Corpus::load(&path).unwrap();

        assert_eq!(
            loaded.examples.first().and_then(|e| e.source.clone()),
            Some("developer-platform".to_string())
        );
        assert_eq!(loaded.examples.get(1).and_then(|e| e.source.clone()), None);
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
