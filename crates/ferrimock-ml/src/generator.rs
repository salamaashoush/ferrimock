//! Synthesising a labelled corpus.
//!
//! Every example here is labelled by the thing that made it. The generator
//! decided to emit an email address, so the example is an email address -- the
//! label is not an opinion about the value, it is a record of the value's
//! origin.
//!
//! That is the difference from the attempt before this one, which asked the
//! built-in detector to label synthetic data and then measured the resulting
//! model against the detector. A student cannot outscore the teacher that graded
//! the exam, and the scores said so without anyone noticing what they meant.
//!
//! ## What makes a corpus wide
//!
//! Not its row count. A million rows drawn from one API, in one language, with
//! one identifier shape, teach exactly as much as the few hundred distinct
//! things they were drawn from. So generation is organised around the axes that
//! actually vary between services:
//!
//! - the [`dialect`] a field belongs to: seventeen families of naming, id and
//!   date conventions, modelled on the house styles real APIs are written in;
//! - the [`lexicon`] its text is written in: twenty locales, most of them not
//!   ASCII;
//! - the [`names`] it goes by, including the ones that say nothing and the few
//!   that say something false;
//! - the [`values`] it holds, drawn from twenty-one identifier shapes and
//!   sixteen date formats;
//! - the [`noise`] a recording carries: empty samples, redactions, truncations,
//!   placeholders.
//!
//! [`census`] counts what came out, per label, so "wide" is a measurement rather
//! than a claim.
//!
//! ## Reproducible, and addressable by index
//!
//! An example is a pure function of its index and the corpus seed. Nothing is
//! held in memory to reproduce a row, which is what makes a corpus of millions
//! streamable, shardable across machines, and identical when regenerated.
//!
//! ## What synthetic data still cannot do
//!
//! It can only contain what someone thought to generate. Every convention here
//! is one that was written down, and a model trained on it has learned this
//! table as much as it has learned the domain. [`crate::eval::ShipGate`] is what
//! stops that from being mistaken for evidence: a model still cannot ship on
//! generated data, however wide the generation.
//!
//! The nearest honest approximation of "does this work on an API it has never
//! seen" is to hold an entire family out of training and score on it --
//! [`Recipe::without`] and [`crate::eval::per_source`] are that experiment.

pub mod census;
pub mod dialect;
pub mod lexicon;
pub mod names;
pub mod noise;
pub mod rng;
pub mod values;

use crate::corpus::{Corpus, Example, Provenance, ValueKind};
use crate::label::FieldLabel;
use dialect::ApiDialect;
use rng::Rng;
use std::path::Path;

/// How many samples a field carries, weighted towards the few that a recording
/// usually has. The tail matters: agreement features say nothing about a field
/// seen once, and everything about one seen ten times.
const SAMPLE_COUNT_WEIGHTS: [u32; 10] = [14, 16, 16, 14, 12, 9, 7, 5, 4, 3];

/// A corpus, described rather than materialised.
///
/// Holds no examples: it is the rule for producing row `n`, which is what lets
/// the same description answer for a thousand rows or ten million.
#[derive(Debug, Clone)]
pub struct Recipe {
    /// How many examples the corpus holds.
    pub count: usize,
    pub seed: u64,
    /// The families drawn from. Narrowing this is how a family is held out of
    /// training so it can be scored as an API the model has never seen.
    pub dialects: Vec<ApiDialect>,
}

impl Recipe {
    /// A corpus of `count` examples drawn from every family.
    pub fn new(count: usize, seed: u64) -> Self {
        Self {
            count,
            seed,
            dialects: ApiDialect::ALL.to_vec(),
        }
    }

    /// `per_label` examples of each label, from every family.
    pub fn balanced(per_label: usize, seed: u64) -> Self {
        Self::new(per_label * FieldLabel::ALL.len(), seed)
    }

    /// The same corpus with one family left out.
    #[must_use]
    pub fn without(mut self, held_out: ApiDialect) -> Self {
        self.dialects.retain(|dialect| *dialect != held_out);
        if self.dialects.is_empty() {
            self.dialects = vec![held_out];
        }
        self
    }

    /// The same corpus drawn only from the families named.
    #[must_use]
    pub fn only(mut self, dialects: Vec<ApiDialect>) -> Self {
        if !dialects.is_empty() {
            self.dialects = dialects;
        }
        self
    }

    /// The example at `index`.
    ///
    /// A pure function of the index and the seed, which is the whole basis of
    /// streaming: nothing before row `n` has to exist for row `n` to.
    pub fn example(&self, index: u64) -> Example {
        let labels = FieldLabel::ALL.len() as u64;
        let families = self.dialects.len().max(1) as u64;

        // The label turns over fastest so that any prefix of the corpus is
        // balanced across labels, and the family turns over once per lap so that
        // each label meets every family in turn.
        #[allow(clippy::cast_possible_truncation)] // both are moduli of small counts
        let (label_index, dialect_index) = (
            (index % labels) as usize,
            ((index / labels) % families) as usize,
        );
        let label = FieldLabel::from_class_index(label_index).unwrap_or(FieldLabel::Opaque);
        let dialect = self
            .dialects
            .get(dialect_index)
            .copied()
            .unwrap_or(ApiDialect::MixedLegacy);

        let mut rng = Rng::for_index(self.seed, index);
        build(label, dialect, &mut rng)
    }

    /// Every example, produced as it is asked for.
    pub fn iter(&self) -> impl Iterator<Item = Example> + '_ {
        (0..self.count as u64).map(|index| self.example(index))
    }

    /// Every example, materialised.
    ///
    /// Fine for the sizes a model is fitted on interactively. For millions,
    /// prefer [`Self::iter`] or [`Self::write_jsonl`], which hold one example at
    /// a time.
    pub fn corpus(&self) -> Corpus {
        Corpus::new(self.iter().collect())
    }

    /// Write the corpus as JSON Lines, one example at a time.
    ///
    /// Streamed rather than collected: a corpus of millions is larger than it is
    /// worth holding in memory, and the point of an indexable recipe is that it
    /// never has to be.
    pub fn write_jsonl(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        use std::io::{BufWriter, Write};

        let file = std::fs::File::create(path)?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        for example in self.iter() {
            serde_json::to_writer(&mut out, &example).map_err(std::io::Error::other)?;
            out.write_all(b"\n")?;
        }
        out.flush()
    }

    /// A census over the first `sample` examples.
    ///
    /// Reading the whole of a large corpus to count its shapes costs as much as
    /// generating it; a prefix is balanced across labels by construction and
    /// says the same thing.
    pub fn census(&self, sample: usize) -> census::Census {
        let taken = sample.min(self.count);
        census::Census::of(&Corpus::new(
            (0..taken as u64).map(|index| self.example(index)).collect(),
        ))
    }
}

/// One example: a name, the values it was seen holding, and what made them.
fn build(label: FieldLabel, dialect: ApiDialect, rng: &mut Rng) -> Example {
    let name = names::draw(label, dialect, rng);
    let style = values::FieldStyle::draw(dialect, label, name.informative, rng);

    let samples = rng.weighted(&SAMPLE_COUNT_WEIGHTS) + 1;
    let mut drawn: Vec<String> = (0..samples)
        .map(|_| values::value(label, style, rng))
        .collect();
    noise::disturb(&mut drawn, dialect.noise(), rng);

    let kind = kind_of(label, &drawn, rng);
    Example::new(name.text, drawn, label, Provenance::Generated)
        .from_source(dialect.name())
        .of_kind(kind)
}

/// The JSON kind a field of this label was recorded as.
///
/// Not a detail. A count and a numeric string id are the same digits, and the
/// quotes around them are the only thing that tells them apart -- so a corpus
/// that does not record this is asking the detector a question with no answer.
fn kind_of(label: FieldLabel, values: &[String], rng: &mut Rng) -> ValueKind {
    match label {
        // A flag is a JSON boolean only when it is spelled as one. `yes`, `Y` and
        // `True` are strings, and that is why the detector has to read them.
        FieldLabel::Boolean => {
            if values
                .iter()
                .all(|value| value == "true" || value == "false")
            {
                ValueKind::Boolean
            } else {
                ValueKind::String
            }
        }
        FieldLabel::UnixTimestamp => ValueKind::Number,
        // Most counts are numbers, and a minority come back quoted -- a price as
        // `"12.50"`, a total as `"42"`. Both happen, and a corpus with only the
        // first teaches that digits are always a number.
        FieldLabel::Number => {
            if rng.chance(5, 6) {
                ValueKind::Number
            } else {
                ValueKind::String
            }
        }
        // Everything else is text, including the identifiers made of digits --
        // which is the whole reason they are identifiers rather than numbers.
        _ => ValueKind::String,
    }
}

/// `per_label` examples of each label, from every family.
pub fn generate_corpus(per_label: usize, seed: u64) -> Corpus {
    Recipe::balanced(per_label, seed).corpus()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    #[test]
    fn every_label_is_represented_equally() {
        let corpus = generate_corpus(20, 1);
        let counts = corpus.label_counts();

        assert_eq!(counts.len(), FieldLabel::ALL.len());
        assert!(
            counts.values().all(|count| *count == 20),
            "labels came out uneven: {counts:?}"
        );
    }

    #[test]
    fn every_family_is_represented() {
        let corpus = generate_corpus(40, 2);
        let sources: FxHashSet<&str> = corpus
            .examples
            .iter()
            .filter_map(|example| example.source.as_deref())
            .collect();
        assert_eq!(
            sources.len(),
            ApiDialect::ALL.len(),
            "a family never appeared: {sources:?}"
        );
    }

    #[test]
    fn a_held_out_family_never_appears() {
        // The basis of the only honest test of generalisation there is: train
        // without a family, then score on it.
        let recipe = Recipe::new(4_000, 3).without(ApiDialect::PaymentsPlatform);
        assert!(
            recipe.iter().all(|example| {
                example.source.as_deref() != Some(ApiDialect::PaymentsPlatform.name())
            }),
            "the held-out family leaked into training"
        );

        let only = Recipe::new(500, 3).only(vec![ApiDialect::PaymentsPlatform]);
        assert!(only.iter().all(|example| {
            example.source.as_deref() == Some(ApiDialect::PaymentsPlatform.name())
        }));
    }

    #[test]
    fn holding_every_family_out_leaves_one_rather_than_nothing() {
        // Silently producing an empty corpus would read as a model that scored
        // perfectly on no data.
        let mut recipe = Recipe::new(100, 1);
        for dialect in ApiDialect::ALL {
            recipe = recipe.without(dialect);
        }
        assert_eq!(recipe.dialects.len(), 1);
        assert_eq!(recipe.iter().count(), 100);
    }

    #[test]
    fn a_row_is_a_function_of_its_index_and_seed_alone() {
        let recipe = Recipe::new(1_000_000, 42);
        let far = recipe.example(999_999);
        let again = recipe.example(999_999);

        assert_eq!(far.field_name, again.field_name);
        assert_eq!(far.values, again.values);
        assert_eq!(far.label, again.label);
        assert_eq!(far.source, again.source);
    }

    #[test]
    fn the_same_seed_gives_the_same_corpus_and_a_different_one_does_not() {
        let render = |corpus: &Corpus| -> Vec<String> {
            corpus
                .examples
                .iter()
                .map(|example| format!("{}={}", example.field_name, example.values.join(",")))
                .collect()
        };

        assert_eq!(
            render(&generate_corpus(5, 42)),
            render(&generate_corpus(5, 42))
        );
        assert_ne!(
            render(&generate_corpus(5, 1)),
            render(&generate_corpus(5, 2))
        );
    }

    #[test]
    fn a_prefix_of_a_large_corpus_is_already_balanced() {
        // What makes a census over a sample worth reading, and what makes a
        // streamed corpus safe to stop early.
        let recipe = Recipe::new(10_000_000, 7);
        let prefix = Corpus::new((0..2_900_u64).map(|index| recipe.example(index)).collect());
        let counts = prefix.label_counts();

        assert_eq!(counts.len(), FieldLabel::ALL.len());
        assert!(counts.values().all(|count| *count == 100), "{counts:?}");
    }

    #[test]
    fn the_corpus_is_wide_along_every_axis_it_claims_to_be() {
        // The claim this whole module exists to make, stated as a measurement
        // rather than left to the reader.
        let census = Recipe::new(60_000, 11).census(60_000);

        assert!(
            census.distinct_names > 1_500,
            "only {} distinct field names",
            census.distinct_names
        );
        assert!(
            census.distinct_shapes > 4_000,
            "only {} distinct value shapes",
            census.distinct_shapes
        );
        assert_eq!(census.sources.len(), ApiDialect::ALL.len());
        assert!(
            census.non_ascii_share > 0.02,
            "only {:.3} of values are non-ASCII",
            census.non_ascii_share
        );
        assert!(
            census.placeholder_share > 0.002,
            "only {:.4} of values are empty or a placeholder",
            census.placeholder_share
        );
        assert!(
            census.mean_samples > 2.0,
            "fields carry only {:.1} samples on average",
            census.mean_samples
        );
    }

    #[test]
    fn no_label_is_covered_by_only_a_handful_of_names_or_shapes() {
        // A label reachable by four names and two shapes has been memorised
        // rather than learned, however many rows carry it. The shape floor is
        // the lower of the two because some labels have few shapes to have: a
        // country code is two letters, and covering both cases is covering all
        // of it.
        let census = Recipe::new(60_000, 13).census(60_000);
        let thin = census.thin(25, 15);
        assert!(
            thin.is_empty(),
            "these labels are too narrowly covered: {:?}",
            thin.iter()
                .map(|(label, coverage)| format!(
                    "{} ({} names, {} shapes)",
                    label.name(),
                    coverage.distinct_names,
                    coverage.distinct_shapes
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn generation_is_fast_enough_that_millions_is_a_real_option() {
        // Not a benchmark -- a guard. If a row ever costs a millisecond, the
        // streaming design stops being usable and nobody finds out until they
        // ask for ten million.
        let recipe = Recipe::new(20_000, 17);
        let started = std::time::Instant::now();
        let produced = recipe.iter().count();
        let elapsed = started.elapsed();

        assert_eq!(produced, 20_000);
        assert!(
            elapsed.as_secs_f64() < 6.0,
            "20k rows took {elapsed:?}, so a million would take minutes"
        );
    }

    #[test]
    fn a_streamed_corpus_reads_back_as_the_one_that_was_written() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corpus.jsonl");

        let recipe = Recipe::new(500, 19);
        recipe.write_jsonl(&path).unwrap();
        let loaded = Corpus::load(&path).unwrap();
        let generated = recipe.corpus();

        assert_eq!(loaded.len(), generated.len());
        for (read, made) in loaded.examples.iter().zip(generated.examples.iter()) {
            assert_eq!(read.field_name, made.field_name);
            assert_eq!(read.values, made.values);
            assert_eq!(read.label, made.label);
            assert_eq!(read.source, made.source);
        }
    }

    #[test]
    fn a_field_records_the_json_kind_it_was_drawn_as() {
        // The evidence a numeric string id and a count differ by, and the only
        // place it can live: not in the text, which is identical.
        let corpus = generate_corpus(60, 21);
        let kind_of_label = |label: FieldLabel| -> Vec<ValueKind> {
            let mut kinds: Vec<ValueKind> = corpus
                .examples
                .iter()
                .filter(|example| example.label == label)
                .map(|example| example.kind)
                .collect();
            kinds.sort_by_key(|kind| format!("{kind:?}"));
            kinds.dedup();
            kinds
        };

        assert_eq!(
            kind_of_label(FieldLabel::NumericStringId),
            vec![ValueKind::String],
            "a numeric string id is text, or it is not one"
        );
        assert_eq!(
            kind_of_label(FieldLabel::UnixTimestamp),
            vec![ValueKind::Number]
        );
        assert!(
            kind_of_label(FieldLabel::Number).contains(&ValueKind::Number),
            "most counts are numbers"
        );
        assert!(
            kind_of_label(FieldLabel::Boolean).contains(&ValueKind::String),
            "`yes` and `Y` are flags spelled as text, and the corpus has to carry them"
        );
    }

    #[test]
    fn a_flag_is_a_json_boolean_only_when_it_is_spelled_as_one() {
        let corpus = generate_corpus(120, 23);
        for example in corpus
            .examples
            .iter()
            .filter(|example| example.label == FieldLabel::Boolean)
        {
            let spelled_as_json = example
                .values
                .iter()
                .all(|value| value == "true" || value == "false");
            assert_eq!(
                example.kind == ValueKind::Boolean,
                spelled_as_json,
                "{:?} was recorded as {:?}",
                example.values,
                example.kind
            );
        }
    }

    #[test]
    fn some_fields_are_named_uninformatively() {
        // Otherwise the corpus teaches a model that the name always gives it
        // away, which real traffic immediately disproves.
        let corpus = generate_corpus(40, 3);
        let vague = corpus
            .examples
            .iter()
            .filter(|example| {
                ["value", "data", "field", "attr", "v", "item", "payload"]
                    .contains(&example.field_name.as_str())
            })
            .count();

        assert!(
            vague > 0,
            "no example forces the model to look at the values"
        );
    }

    #[test]
    fn a_field_carries_more_than_one_sample_most_of_the_time() {
        let corpus = generate_corpus(30, 11);
        let multi = corpus
            .examples
            .iter()
            .filter(|example| example.values.len() > 1)
            .count();
        assert!(
            multi * 2 > corpus.len(),
            "only {multi} of {} fields carry more than one sample, and agreement features \
             say nothing about the rest",
            corpus.len()
        );
    }
}
