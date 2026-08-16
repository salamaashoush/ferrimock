//! Finding defects in the engine's own detector.
//!
//! This is what the corpus and the model are actually for. Neither one ships.
//! The corpus knows what every field really is, because a generator decided it;
//! the detector is asked the same question and its wrong answers are collected,
//! counted and ranked. That list is a defect list for the imperative detector,
//! written in the detector's own vocabulary -- "a ULID came back as
//! `random_string`, 4,213 times, here are ten of them".
//!
//! The model's role is to say how much of the gap is worth chasing. A detector
//! that scores 0.61 where a linear model on the same rows scores 0.98 is leaving
//! signal on the floor, and the signal is recoverable by rules. A detector that
//! scores 0.61 where the model scores 0.62 is meeting the noise floor, and the
//! remaining errors are not defects at all -- they are fields no reader could
//! place. Without the second number there is no way to tell those apart, and
//! effort goes into the cases that were never winnable.
//!
//! What the model is *not* is a thing to run at consolidation time. Every defect
//! it exposes is meant to be closed in the detector, after which the model is
//! only useful for finding the next one.

use crate::corpus::{Corpus, Example};
use crate::detector::{detect, kind_of};
use crate::eval::{Evaluation, evaluate};
use crate::label::FieldLabel;
use crate::linear::LinearClassifier;
use crate::{Classifier, HeuristicClassifier};
use ferrimock::type_detector::TypeDetector;
use rustc_hash::FxHashMap;
use std::fmt::Write;

/// One way the detector is wrong, and how often.
#[derive(Debug, Clone)]
pub struct Defect {
    /// What the field actually is.
    pub truth: FieldLabel,
    /// The kind the detector answered with, in its own vocabulary.
    pub detector_said: &'static str,
    pub count: usize,
    /// Share of every field of this label that the detector answers this way.
    pub share_of_label: f64,
    /// Concrete fields, for reproducing it.
    pub examples: Vec<Example>,
    /// Share of these same rows a fitted model gets right.
    ///
    /// The number that decides whether the defect is worth fixing: near one and
    /// the signal is there to be read, near zero and nothing could have read it.
    pub recoverable: f64,
}

/// What an audit found.
#[derive(Debug, Clone)]
pub struct Audit {
    pub examples: usize,
    pub detector: Evaluation,
    pub model: Evaluation,
    pub defects: Vec<Defect>,
}

impl Audit {
    /// How much of what the detector misses a model on the same rows recovers.
    pub fn headroom(&self) -> f64 {
        self.model.macro_f1() - self.detector.macro_f1()
    }
}

/// Run the detector over a corpus and collect everything it gets wrong.
///
/// `model` is scored on the same rows so each defect can be marked recoverable
/// or not. `examples_per_defect` bounds how many concrete fields are kept.
pub fn audit(corpus: &Corpus, model: &dyn Classifier, examples_per_defect: usize) -> Audit {
    let detector = TypeDetector::new();
    let mut grouped: FxHashMap<(FieldLabel, &'static str), (usize, Vec<Example>, usize)> =
        FxHashMap::default();
    let mut per_label: FxHashMap<FieldLabel, usize> = FxHashMap::default();

    for example in &corpus.examples {
        *per_label.entry(example.label).or_insert(0) += 1;

        let values = example.value_refs();
        let field = example.as_field(&values);
        let (field_type, _) = detect(&detector, &field);
        let kind = kind_of(&field_type);

        // The detector is right when its answer projects onto the true label.
        // Structural answers project onto nothing, and are wrong here for the
        // same reason: a ULID is not an array.
        if FieldLabel::from_field_type(&field_type) == Some(example.label) {
            continue;
        }

        let entry = grouped
            .entry((example.label, kind))
            .or_insert_with(|| (0, Vec::new(), 0));
        entry.0 += 1;
        if entry.1.len() < examples_per_defect {
            entry.1.push(example.clone());
        }
        if model
            .classify(&field)
            .is_some_and(|(predicted, _)| predicted == example.label)
        {
            entry.2 += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)] // corpus counts are far below f64's exact range
    let mut defects: Vec<Defect> = grouped
        .into_iter()
        .map(
            |((truth, detector_said), (count, examples, recovered))| Defect {
                truth,
                detector_said,
                count,
                share_of_label: per_label
                    .get(&truth)
                    .map_or(0.0, |support| count as f64 / *support as f64),
                examples,
                recoverable: if count == 0 {
                    0.0
                } else {
                    recovered as f64 / count as f64
                },
            },
        )
        .collect();

    // Most frequent first, because that is the order they are worth fixing in.
    defects.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.truth.cmp(&right.truth))
    });

    Audit {
        examples: corpus.len(),
        detector: evaluate(&HeuristicClassifier::new(), corpus),
        model: evaluate(model, corpus),
        defects,
    }
}

/// Render an audit as something a fixer can work from.
///
/// `hints` is the fitted linear model, read for the features it leans on per
/// class. Those are the rules to encode: a weight on `value.crockford_only` for
/// a class the detector answers `random_string` on is the detector telling you,
/// through the model, exactly which check it is missing.
pub fn report(audit: &Audit, hints: &LinearClassifier, limit: usize) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "detector audit over {} fields\n", audit.examples);
    let _ = writeln!(
        out,
        "  detector macro-F1 {:.3}   a linear model on the same rows {:.3}",
        audit.detector.macro_f1(),
        audit.model.macro_f1()
    );
    let _ = writeln!(
        out,
        "  headroom {:+.3} -- signal the detector could read and does not\n",
        audit.headroom()
    );

    let _ = writeln!(out, "worst classes for the detector:");
    let _ = writeln!(
        out,
        "  {:<20} {:>8} {:>7} {:>7} {:>10} {:>10}",
        "label", "support", "P", "R", "detector F1", "model F1"
    );
    let mut classes: Vec<(&FieldLabel, f64, f64)> = audit
        .detector
        .per_class
        .iter()
        .filter(|(_, score)| score.support > 0)
        .map(|(label, score)| {
            let model_f1 = audit
                .model
                .per_class
                .get(label)
                .map_or(0.0, crate::eval::ClassScore::f1);
            (label, score.f1(), model_f1)
        })
        .collect();
    classes.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (label, detector_f1, model_f1) in classes.iter().take(limit) {
        let score = audit
            .detector
            .per_class
            .get(*label)
            .copied()
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {:<20} {:>8} {:>7.3} {:>7.3} {:>10.3} {:>10.3}",
            label.name(),
            score.support,
            score.precision(),
            score.recall(),
            detector_f1,
            model_f1
        );
    }

    let _ = writeln!(out, "\ndefects, most frequent first:\n");
    for (rank, defect) in audit.defects.iter().take(limit).enumerate() {
        let verdict = if defect.recoverable >= 0.8 {
            "recoverable: a model reads this right almost always, so a rule can"
        } else if defect.recoverable >= 0.4 {
            "partly recoverable: a model does better here but not reliably"
        } else {
            "not a defect: nothing reads this right, so the field is genuinely ambiguous"
        };

        let _ = writeln!(
            out,
            "  [{}] {}x  {} read as `{}`  ({:.0}% of all {})",
            rank + 1,
            defect.count,
            defect.truth.name(),
            defect.detector_said,
            defect.share_of_label * 100.0,
            defect.truth.name()
        );
        let _ = writeln!(out, "      {verdict} ({:.0}%)", defect.recoverable * 100.0);

        for example in &defect.examples {
            let shown: Vec<&str> = example.value_refs().into_iter().take(2).collect();
            let _ = writeln!(
                out,
                "      {:<24} {}",
                truncate(&example.field_name, 24),
                shown
                    .iter()
                    .map(|value| truncate(value, 44))
                    .collect::<Vec<_>>()
                    .join("   ")
            );
        }

        if defect.recoverable >= 0.4 {
            let leaned_on = hints.explain(defect.truth, 4);
            if !leaned_on.is_empty() {
                let _ = writeln!(
                    out,
                    "      the signal a model uses for {}:",
                    defect.truth.name()
                );
                for (feature, weight) in leaned_on {
                    let _ = writeln!(out, "        {weight:+.3}  {feature}");
                }
            }
        }
        out.push('\n');
    }

    out
}

fn truncate(text: &str, width: usize) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= width {
        return text.to_string();
    }
    let kept: String = characters
        .into_iter()
        .take(width.saturating_sub(1))
        .collect();
    format!("{kept}\u{2026}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::corpus::Provenance;
    use crate::linear::TrainingConfig;

    fn example(name: &str, values: &[&str], label: FieldLabel) -> Example {
        Example::new(
            name,
            values.iter().map(|value| (*value).to_string()).collect(),
            label,
            Provenance::Generated,
        )
    }

    /// Answers `opaque` to everything, so nothing it is shown is ever recovered.
    struct Perfect;
    impl Classifier for Perfect {
        fn name(&self) -> &str {
            "perfect"
        }
        fn classify(&self, _: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
            Some((FieldLabel::Opaque, 0.99))
        }
    }

    /// Reads a timestamp correctly, so everything it is shown is recoverable.
    struct Reads;
    impl Classifier for Reads {
        fn name(&self) -> &str {
            "reads"
        }
        fn classify(&self, _: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
            Some((FieldLabel::Timestamp, 0.99))
        }
    }

    #[test]
    fn a_defect_names_what_the_detector_actually_said() {
        // The report has to be written in the detector's vocabulary, because
        // that is the vocabulary the fix is written in. Here a ULID is answered
        // `token`, and "wrong" would not tell anybody where to look.
        let corpus = Corpus::new(vec![
            example(
                "reference",
                &["01ARZ3NDEKTSV4RRFFQ69G5FAV", "01BX5ZZKBKACTAV9WEVGEMMVRZ"],
                FieldLabel::Opaque,
            );
            20
        ]);

        let defect = audit(&corpus, &Perfect, 3)
            .defects
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(defect.truth, FieldLabel::Opaque);
        assert_eq!(defect.detector_said, "token");
        assert_eq!(defect.count, 20);
    }

    #[test]
    fn a_mistake_is_grouped_counted_and_illustrated() {
        // An ISO date labelled as a timestamp: the detector answers `iso_date`,
        // which is the wrong label and a nameable one.
        let corpus = Corpus::new(vec![
            example(
                "occurred",
                &["2024-03-17", "2019-11-02"],
                FieldLabel::Timestamp
            );
            12
        ]);

        let audit = audit(&corpus, &Perfect, 3);
        let defect = audit.defects.first().unwrap();

        assert_eq!(defect.truth, FieldLabel::Timestamp);
        assert_eq!(defect.detector_said, "iso_date");
        assert_eq!(defect.count, 12);
        assert!((defect.share_of_label - 1.0).abs() < 1e-9);
        assert_eq!(defect.examples.len(), 3, "capped at what was asked for");
        assert!(
            defect.recoverable < 0.1,
            "the stand-in model answers opaque, so nothing here is recovered"
        );
    }

    #[test]
    fn defects_are_ranked_by_how_often_they_happen() {
        let mut examples = vec![example("occurred", &["2024-03-17"], FieldLabel::Timestamp); 30];
        examples.extend(vec![example("flag", &["a@b.com"], FieldLabel::Boolean); 5]);
        let audit = audit(&Corpus::new(examples), &Perfect, 2);

        assert!(audit.defects.len() >= 2);
        let counts: Vec<usize> = audit.defects.iter().map(|defect| defect.count).collect();
        assert!(
            counts
                .windows(2)
                .all(|pair| matches!(pair, [earlier, later] if earlier >= later)),
            "a fixer reads this top down: {counts:?}"
        );
    }

    #[test]
    fn a_recoverable_defect_is_told_apart_from_an_ambiguous_one() {
        // The distinction the whole module exists for: a defect a model can read
        // is a missing rule, and one nothing can read is not a defect at all.
        let corpus = Corpus::new(vec![
            example(
                "occurred",
                &["2024-03-17"],
                FieldLabel::Timestamp
            );
            20
        ]);

        let recoverable = audit(&corpus, &Reads, 2);
        assert!((recoverable.defects.first().unwrap().recoverable - 1.0).abs() < 1e-9);

        let ambiguous = audit(&corpus, &Perfect, 2);
        assert!(ambiguous.defects.first().unwrap().recoverable < 0.1);
    }

    #[test]
    fn the_report_says_what_to_fix_and_what_signal_to_fix_it_with() {
        let corpus = crate::generator::Recipe::balanced(30, 4).corpus();
        let model = LinearClassifier::train(&corpus, TrainingConfig::default());
        let audit = audit(&corpus, &model, 3);
        let report = report(&audit, &model, 5);

        assert!(report.contains("headroom"), "{report}");
        assert!(report.contains("defects, most frequent first"), "{report}");
        assert!(
            audit.headroom() > 0.0,
            "a model fitted on this corpus should beat the detector on it"
        );
    }

    #[test]
    fn a_long_value_is_shortened_rather_than_wrapped_across_the_report() {
        assert_eq!(truncate("short", 24), "short");
        let long = truncate(&"x".repeat(80), 10);
        assert_eq!(long.chars().count(), 10);
        assert!(long.ends_with('\u{2026}'));
        // Character-wise, so a Japanese file name is not cut in half.
        assert_eq!(truncate("東京都渋谷区", 3).chars().count(), 3);
    }
}
