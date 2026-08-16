//! Counting what a corpus actually contains.
//!
//! Row count is the least interesting number a corpus has. Ten million rows
//! drawn from one API, in one language, with one identifier shape, teach exactly
//! as much as the few hundred distinct things they were drawn from -- and read
//! as a far more impressive corpus than they are.
//!
//! So the number that matters is how many *distinct shapes* the rows cover, and
//! a census reports it per label. It is also the tool for widening the corpus:
//! [`Census::thin`] names the labels that are covered by too few spellings or
//! too few value shapes, which is the list of what to write next.

use crate::corpus::Corpus;
use crate::label::FieldLabel;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt::Write;

/// A value reduced to the pattern it is made of.
///
/// Re-exported from [`crate::shape`], which feature extraction reads too: the
/// census counts how many distinct shapes a corpus covers, and a model asks
/// whether a field's samples share one.
pub use crate::shape::signature as shape_signature;

/// What one label is covered by.
#[derive(Debug, Clone, Default)]
pub struct LabelCoverage {
    pub examples: usize,
    pub distinct_names: usize,
    pub distinct_shapes: usize,
    /// Share of this label's values written outside ASCII.
    pub non_ascii_share: f64,
}

/// What a corpus contains, past its row count.
#[derive(Debug, Clone)]
pub struct Census {
    pub examples: usize,
    pub values: usize,
    pub distinct_names: usize,
    pub distinct_shapes: usize,
    /// The API families represented, by name.
    pub sources: Vec<String>,
    pub per_label: Vec<(FieldLabel, LabelCoverage)>,
    pub mean_samples: f64,
    /// Share of all values written outside ASCII.
    pub non_ascii_share: f64,
    /// Share of all values that are empty or a stand-in for a missing one.
    pub placeholder_share: f64,
    pub reviewed: usize,
}

/// What is being accumulated for one label while a census runs.
#[derive(Default)]
struct LabelTally<'a> {
    examples: usize,
    names: FxHashSet<&'a str>,
    shapes: FxHashSet<String>,
    non_ascii_values: usize,
    values: usize,
}

impl Census {
    /// Take a census of a corpus.
    pub fn of(corpus: &Corpus) -> Self {
        let mut names: FxHashSet<&str> = FxHashSet::default();
        let mut shapes: FxHashSet<String> = FxHashSet::default();
        let mut sources: FxHashSet<&str> = FxHashSet::default();
        let mut per_label: FxHashMap<FieldLabel, LabelTally<'_>> = FxHashMap::default();

        let mut values = 0_usize;
        let mut non_ascii = 0_usize;
        let mut placeholders = 0_usize;
        let mut reviewed = 0_usize;

        for example in &corpus.examples {
            names.insert(example.field_name.as_str());
            if let Some(source) = example.source.as_deref() {
                sources.insert(source);
            }
            if example.provenance == crate::corpus::Provenance::Reviewed {
                reviewed += 1;
            }

            let tally = per_label.entry(example.label).or_default();
            tally.examples += 1;
            tally.names.insert(example.field_name.as_str());

            for value in &example.values {
                values += 1;
                tally.values += 1;
                let signature = shape_signature(value);
                shapes.insert(signature.clone());
                tally.shapes.insert(signature);
                if !value.is_ascii() {
                    non_ascii += 1;
                    tally.non_ascii_values += 1;
                }
                if is_placeholder(value) {
                    placeholders += 1;
                }
            }
        }

        let mut coverage: Vec<(FieldLabel, LabelCoverage)> = per_label
            .into_iter()
            .map(|(label, tally)| {
                (
                    label,
                    LabelCoverage {
                        examples: tally.examples,
                        distinct_names: tally.names.len(),
                        distinct_shapes: tally.shapes.len(),
                        non_ascii_share: share(tally.non_ascii_values, tally.values),
                    },
                )
            })
            .collect();
        coverage.sort_by_key(|(label, _)| *label);

        let mut source_names: Vec<String> = sources.into_iter().map(str::to_string).collect();
        source_names.sort();

        Self {
            examples: corpus.len(),
            values,
            distinct_names: names.len(),
            distinct_shapes: shapes.len(),
            sources: source_names,
            per_label: coverage,
            mean_samples: share(values, corpus.len()),
            non_ascii_share: share(non_ascii, values),
            placeholder_share: share(placeholders, values),
            reviewed,
        }
    }

    /// Labels covered by too few spellings or too few value shapes.
    ///
    /// The list of what to widen next. A label reachable by four names and two
    /// shapes has been memorised rather than learned, however many rows carry
    /// it.
    pub fn thin(&self, min_names: usize, min_shapes: usize) -> Vec<(FieldLabel, LabelCoverage)> {
        self.per_label
            .iter()
            .filter(|(_, coverage)| {
                coverage.distinct_names < min_names || coverage.distinct_shapes < min_shapes
            })
            .map(|(label, coverage)| (*label, coverage.clone()))
            .collect()
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} examples, {} values ({:.1} per field), {} reviewed",
            self.examples, self.values, self.mean_samples, self.reviewed
        );
        let _ = writeln!(
            out,
            "{} distinct field names, {} distinct value shapes, {} API families",
            self.distinct_names,
            self.distinct_shapes,
            self.sources.len()
        );
        let _ = writeln!(
            out,
            "{:.1}% of values are written outside ASCII, {:.1}% are empty or a placeholder",
            self.non_ascii_share * 100.0,
            self.placeholder_share * 100.0
        );
        out.push_str("\nper label:\n");
        let _ = writeln!(
            out,
            "  {:<20} {:>9} {:>7} {:>8} {:>10}",
            "label", "examples", "names", "shapes", "non-ASCII"
        );
        for (label, coverage) in &self.per_label {
            let _ = writeln!(
                out,
                "  {:<20} {:>9} {:>7} {:>8} {:>9.1}%",
                label.name(),
                coverage.examples,
                coverage.distinct_names,
                coverage.distinct_shapes,
                coverage.non_ascii_share * 100.0
            );
        }
        out
    }
}

/// Whether a value is a stand-in rather than a value.
fn is_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lowered = trimmed.to_lowercase();
    matches!(
        lowered.as_str(),
        "n/a" | "-" | "--" | "unknown" | "null" | "none" | "tbd" | "not set"
    ) || trimmed.starts_with('*')
        || trimmed.starts_with('[')
        || trimmed.starts_with('<')
}

#[allow(clippy::cast_precision_loss)] // corpus counts are far below f64's exact range
fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::corpus::{Example, Provenance};

    #[test]
    fn a_census_counts_shapes_rather_than_rows() {
        let repeated: Vec<Example> = (0..1000)
            .map(|_| {
                Example::new(
                    "id",
                    vec!["2024-03-17".to_string()],
                    FieldLabel::IsoDate,
                    Provenance::Generated,
                )
            })
            .collect();
        let census = Census::of(&Corpus::new(repeated));

        assert_eq!(census.examples, 1000);
        assert_eq!(
            census.distinct_shapes, 1,
            "a thousand identical rows are one shape, and the census has to say so"
        );
        assert_eq!(census.distinct_names, 1);
    }

    #[test]
    fn thin_labels_are_the_ones_that_need_widening() {
        let examples = vec![
            Example::new(
                "email",
                vec!["a@b.com".to_string()],
                FieldLabel::Email,
                Provenance::Generated,
            ),
            Example::new(
                "id",
                vec!["1".to_string()],
                FieldLabel::Number,
                Provenance::Generated,
            ),
        ];
        let census = Census::of(&Corpus::new(examples));
        let thin = census.thin(5, 5);

        assert_eq!(
            thin.len(),
            2,
            "both labels are covered by one name and one shape"
        );
        assert!(census.report().contains("email"));
    }

    #[test]
    fn a_placeholder_is_counted_as_one() {
        let examples = vec![Example::new(
            "note",
            vec![
                "a real value".to_string(),
                String::new(),
                "N/A".to_string(),
                "[REDACTED]".to_string(),
            ],
            FieldLabel::Sentence,
            Provenance::Generated,
        )];
        let census = Census::of(&Corpus::new(examples));

        assert!(
            (census.placeholder_share - 0.75).abs() < 1e-9,
            "{}",
            census.placeholder_share
        );
    }

    #[test]
    fn non_ascii_values_are_counted_per_label_as_well_as_overall() {
        let examples = vec![
            Example::new(
                "name",
                vec!["田中太郎".to_string(), "Ada Lovelace".to_string()],
                FieldLabel::PersonName,
                Provenance::Generated,
            ),
            Example::new(
                "id",
                vec!["12345".to_string()],
                FieldLabel::NumericStringId,
                Provenance::Generated,
            ),
        ];
        let census = Census::of(&Corpus::new(examples));

        let names = census
            .per_label
            .iter()
            .find(|(label, _)| *label == FieldLabel::PersonName)
            .map(|(_, coverage)| coverage.non_ascii_share)
            .unwrap();
        assert!((names - 0.5).abs() < 1e-9, "{names}");
        assert!((census.non_ascii_share - 1.0 / 3.0).abs() < 1e-9);
    }
}
