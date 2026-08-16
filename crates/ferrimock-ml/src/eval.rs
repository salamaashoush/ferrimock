//! Measuring a classifier honestly.
//!
//! Accuracy alone hides everything that matters here. The label distribution is
//! badly skewed -- a corpus is mostly ids and opaque strings -- so a classifier
//! that answers `opaque` to everything can score well while being useless. Macro
//! F1 weights every class alike and does not let that pass.
//!
//! Calibration is measured too, and not as a nicety. The engine picks between a
//! profile's answer and the detector's by comparing confidences, so a model that
//! says 0.9 and is right half the time does not merely look bad -- it wins
//! arguments it should lose.

// Test classifiers here answer `name()` with a literal, which reads as needlessly
// bound against the `&str` the trait must return for owned names elsewhere.
#![allow(clippy::unnecessary_literal_bound)]

use crate::{Classifier, corpus::Corpus, label::FieldLabel};
use rustc_hash::FxHashMap;
use std::fmt::Write;

/// How one classifier did on one corpus.
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub classifier: String,
    /// Examples the classifier was asked about.
    pub total: usize,
    /// Examples it declined to answer. Abstaining is not an error -- the
    /// detector abstains on structural types by design -- but it is not a
    /// success either, so it is counted separately.
    pub abstained: usize,
    pub correct: usize,
    pub per_class: FxHashMap<FieldLabel, ClassScore>,
    /// `(actual, predicted) -> count`, for everything it got wrong.
    pub confusion: FxHashMap<(FieldLabel, FieldLabel), usize>,
    /// Reliability by confidence decile.
    pub calibration: Vec<CalibrationBin>,
}

/// Precision, recall and F1 for a single class.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClassScore {
    pub support: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
}

#[allow(clippy::cast_precision_loss)]
impl ClassScore {
    pub fn precision(&self) -> f64 {
        let predicted = self.true_positives + self.false_positives;
        if predicted == 0 {
            0.0
        } else {
            self.true_positives as f64 / predicted as f64
        }
    }

    pub fn recall(&self) -> f64 {
        let actual = self.true_positives + self.false_negatives;
        if actual == 0 {
            0.0
        } else {
            self.true_positives as f64 / actual as f64
        }
    }

    pub fn f1(&self) -> f64 {
        let (precision, recall) = (self.precision(), self.recall());
        if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        }
    }
}

/// One slice of the reliability curve.
#[derive(Debug, Clone, Copy)]
pub struct CalibrationBin {
    pub lower: f64,
    pub upper: f64,
    pub count: usize,
    /// Mean confidence claimed in this bin.
    pub mean_confidence: f64,
    /// Share actually correct in this bin.
    pub accuracy: f64,
}

#[allow(clippy::cast_precision_loss)]
impl Evaluation {
    /// Share of answered examples that were right.
    pub fn accuracy(&self) -> f64 {
        let answered = self.total - self.abstained;
        if answered == 0 {
            0.0
        } else {
            self.correct as f64 / answered as f64
        }
    }

    /// Share of *all* examples that were right. An abstention counts against it,
    /// which is what makes this the number to compare classifiers on: answering
    /// nothing is not a way to score well.
    pub fn coverage_accuracy(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.correct as f64 / self.total as f64
        }
    }

    /// F1 averaged over classes present in the corpus, each weighted alike.
    pub fn macro_f1(&self) -> f64 {
        let present: Vec<&ClassScore> = self
            .per_class
            .values()
            .filter(|score| score.support > 0)
            .collect();
        if present.is_empty() {
            return 0.0;
        }
        present.iter().map(|score| score.f1()).sum::<f64>() / present.len() as f64
    }

    /// Expected calibration error: how far claimed confidence sits from observed
    /// accuracy, averaged over the bins and weighted by how many landed in each.
    /// Zero is perfect; a model saying 0.9 while being right 0.5 of the time
    /// contributes 0.4.
    pub fn expected_calibration_error(&self) -> f64 {
        let answered = self.total - self.abstained;
        if answered == 0 {
            return 0.0;
        }
        self.calibration
            .iter()
            .map(|bin| {
                let weight = bin.count as f64 / answered as f64;
                weight * (bin.mean_confidence - bin.accuracy).abs()
            })
            .sum()
    }

    /// The mistakes it made most often.
    pub fn top_confusions(&self, limit: usize) -> Vec<((FieldLabel, FieldLabel), usize)> {
        let mut pairs: Vec<((FieldLabel, FieldLabel), usize)> = self
            .confusion
            .iter()
            .map(|(pair, count)| (*pair, *count))
            .collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        pairs.truncate(limit);
        pairs
    }

    /// A human-readable report.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "classifier: {}", self.classifier);
        let _ = writeln!(
            out,
            "  examples {}  answered {}  abstained {}",
            self.total,
            self.total - self.abstained,
            self.abstained
        );
        let _ = writeln!(
            out,
            "  accuracy(answered) {:.3}  accuracy(all) {:.3}  macro-F1 {:.3}  ECE {:.3}",
            self.accuracy(),
            self.coverage_accuracy(),
            self.macro_f1(),
            self.expected_calibration_error()
        );

        let mut classes: Vec<(&FieldLabel, &ClassScore)> = self
            .per_class
            .iter()
            .filter(|(_, score)| score.support > 0)
            .collect();
        classes.sort_by_key(|(label, _)| **label);

        out.push_str("  per class:\n");
        for (label, score) in classes {
            let _ = writeln!(
                out,
                "    {:<20} support {:>5}  P {:.3}  R {:.3}  F1 {:.3}",
                label.name(),
                score.support,
                score.precision(),
                score.recall(),
                score.f1()
            );
        }

        let confusions = self.top_confusions(8);
        if !confusions.is_empty() {
            out.push_str("  most common mistakes:\n");
            for ((actual, predicted), count) in confusions {
                let _ = writeln!(
                    out,
                    "    {:<20} read as {:<20} {:>5}",
                    actual.name(),
                    predicted.name(),
                    count
                );
            }
        }

        out
    }
}

/// Score a classifier over a corpus.
pub fn evaluate(classifier: &dyn Classifier, corpus: &Corpus) -> Evaluation {
    const BINS: usize = 10;

    let mut per_class: FxHashMap<FieldLabel, ClassScore> = FxHashMap::default();
    let mut confusion: FxHashMap<(FieldLabel, FieldLabel), usize> = FxHashMap::default();
    let mut bins = vec![(0usize, 0.0f64, 0usize); BINS];
    let mut abstained = 0;
    let mut correct = 0;

    for example in &corpus.examples {
        per_class.entry(example.label).or_default().support += 1;

        let values = example.value_refs();
        let Some((predicted, confidence)) = classifier.classify(&example.as_field(&values)) else {
            abstained += 1;
            per_class.entry(example.label).or_default().false_negatives += 1;
            continue;
        };

        if predicted == example.label {
            correct += 1;
            per_class.entry(predicted).or_default().true_positives += 1;
        } else {
            per_class.entry(predicted).or_default().false_positives += 1;
            per_class.entry(example.label).or_default().false_negatives += 1;
            *confusion.entry((example.label, predicted)).or_insert(0) += 1;
        }

        let clamped = confidence.clamp(0.0, 1.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )] // `clamped` is in [0,1] and BINS is 10
        let index = ((clamped * BINS as f64) as usize).min(BINS - 1);
        if let Some(bin) = bins.get_mut(index) {
            bin.0 += 1;
            bin.1 += clamped;
            if predicted == example.label {
                bin.2 += 1;
            }
        }
    }

    #[allow(clippy::cast_precision_loss)] // Bin counts are far below f64's exact range
    let calibration = bins
        .into_iter()
        .enumerate()
        .map(|(index, (count, sum, hits))| CalibrationBin {
            lower: index as f64 / BINS as f64,
            upper: (index + 1) as f64 / BINS as f64,
            count,
            mean_confidence: if count == 0 { 0.0 } else { sum / count as f64 },
            accuracy: if count == 0 {
                0.0
            } else {
                hits as f64 / count as f64
            },
        })
        .collect();

    Evaluation {
        classifier: classifier.name().to_string(),
        total: corpus.len(),
        abstained,
        correct,
        per_class,
        confusion,
        calibration,
    }
}

/// How a classifier did on one API.
#[derive(Debug, Clone)]
pub struct SourceScore {
    pub source: String,
    pub examples: usize,
    pub macro_f1: f64,
    pub coverage_accuracy: f64,
}

/// Score a classifier separately on each API the corpus was drawn from.
///
/// An average over the whole corpus hides the case that matters: a model can
/// score 0.99 overall while being useless on one family, and the family it is
/// useless on is the one somebody is about to point it at. The number worth
/// quoting is the worst row here, not the mean.
pub fn per_source(classifier: &dyn Classifier, corpus: &Corpus) -> Vec<SourceScore> {
    corpus
        .sources()
        .into_iter()
        .map(|source| {
            let slice = corpus.only_source(&source);
            let evaluation = evaluate(classifier, &slice);
            SourceScore {
                source,
                examples: slice.len(),
                macro_f1: evaluation.macro_f1(),
                coverage_accuracy: evaluation.coverage_accuracy(),
            }
        })
        .collect()
}

/// What a model scored on an API that was kept out of its training.
///
/// The nearest thing to an answer to "will this work on a service nobody has
/// seen". Everything else measured here is measured on data drawn the same way
/// the training data was, and so says nothing about a convention the corpus does
/// not contain.
#[derive(Debug, Clone)]
pub struct HeldOutScore {
    pub source: String,
    pub examples: usize,
    /// The candidate's macro F1 on the family it never trained on.
    pub candidate: f64,
    /// The built-in detector's, on the same rows. It never trains at all, so it
    /// is the floor a held-out score has to clear to mean anything.
    pub heuristic: f64,
}

impl HeldOutScore {
    pub fn beats_heuristic(&self) -> bool {
        self.candidate > self.heuristic
    }
}

/// Score a classifier on an API held out of its training, against the detector.
pub fn held_out(
    candidate: &dyn Classifier,
    heuristic: &dyn Classifier,
    corpus: &Corpus,
    source: &str,
) -> HeldOutScore {
    let slice = corpus.only_source(source);
    HeldOutScore {
        source: source.to_string(),
        examples: slice.len(),
        candidate: evaluate(candidate, &slice).macro_f1(),
        heuristic: evaluate(heuristic, &slice).macro_f1(),
    }
}

/// Render a set of held-out results, worst first.
///
/// Worst first on purpose: the mean of these is the number that flatters, and
/// the minimum is the number that predicts the next complaint.
pub fn held_out_report(scores: &[HeldOutScore]) -> String {
    let mut ranked: Vec<&HeldOutScore> = scores.iter().collect();
    ranked.sort_by(|left, right| {
        left.candidate
            .partial_cmp(&right.candidate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = String::new();
    let _ = writeln!(
        out,
        "held-out APIs (trained without each, then scored on it), worst first:"
    );
    for score in &ranked {
        let verdict = if score.beats_heuristic() {
            "beats the detector"
        } else {
            "LOSES to the detector"
        };
        let _ = writeln!(
            out,
            "  {:<24} {:>6} examples   candidate {:.3}  detector {:.3}   {verdict}",
            score.source, score.examples, score.candidate, score.heuristic
        );
    }

    if let Some(worst) = ranked.first() {
        #[allow(clippy::cast_precision_loss)] // family counts are tiny
        let mean = scores.iter().map(|score| score.candidate).sum::<f64>() / scores.len() as f64;
        let _ = writeln!(
            out,
            "\n  worst {:.3} on {}, mean {:.3} -- the worst is the one that predicts what \
             happens on an API nobody has measured",
            worst.candidate, worst.source, mean
        );
    }
    out
}

/// Whether a candidate has earned its place.
///
/// Four bars, and the last one is the one that matters most.
///
/// Beating the detector is what makes a model useful. Beating a linear model on
/// the same features is what shows the gain came from the model rather than from
/// feature engineering anyone could have done. Calibration is what lets the
/// engine compare its confidence against the detector's.
///
/// And being measured on real, reviewed traffic is what makes any of those
/// numbers mean anything. A model trained and tested on one generator's output
/// has learned that generator. It will score beautifully and say nothing about
/// how it behaves on a recording. The previous attempt at this had no such bar,
/// scored well, and was worthless -- so this gate cannot be passed without it.
#[derive(Debug, Clone, Copy)]
pub struct ShipGate {
    pub beats_heuristic: bool,
    pub beats_baseline: bool,
    pub calibration_ok: bool,
    pub measured_on_real_data: bool,
}

impl ShipGate {
    /// Worst calibration error a model may ship with. A model whose confidence
    /// is this far out cannot be compared against the detector's, which is the
    /// one thing the engine needs it for.
    pub const MAX_CALIBRATION_ERROR: f64 = 0.15;

    /// Fewest reviewed examples a test split must hold before the numbers over
    /// it are worth quoting.
    pub const MIN_REVIEWED_EXAMPLES: usize = 50;

    /// The classifiers are scored here rather than handed in already scored.
    /// Every bar has to be decided on the reviewed rows of `test` and nothing
    /// else: a caller holding evaluations over the whole split could otherwise
    /// pass those, and a few reviewed rows would unlock a verdict about the
    /// generator's output -- which is the exact failure the reviewed-data bar
    /// exists to catch.
    pub fn assess(
        candidate: &dyn Classifier,
        heuristic: &dyn Classifier,
        baseline: &dyn Classifier,
        test: &Corpus,
    ) -> Self {
        let reviewed = test.reviewed_only();
        let candidate = evaluate(candidate, &reviewed);
        let heuristic = evaluate(heuristic, &reviewed);
        let baseline = evaluate(baseline, &reviewed);

        Self {
            beats_heuristic: candidate.macro_f1() > heuristic.macro_f1(),
            beats_baseline: candidate.macro_f1() > baseline.macro_f1(),
            calibration_ok: candidate.expected_calibration_error() <= Self::MAX_CALIBRATION_ERROR,
            measured_on_real_data: reviewed.len() >= Self::MIN_REVIEWED_EXAMPLES,
        }
    }

    pub fn passed(&self) -> bool {
        self.beats_heuristic
            && self.beats_baseline
            && self.calibration_ok
            && self.measured_on_real_data
    }

    pub fn explain(&self) -> String {
        if self.passed() {
            return "ships: beats the detector and the linear baseline on reviewed real \
                    traffic, and is calibrated"
                .to_string();
        }
        // With too little reviewed traffic the other three bars were computed
        // over nothing worth quoting, so reporting them would invent detail.
        if !self.measured_on_real_data {
            return format!(
                "does not ship: was not measured on at least {} reviewed examples of real \
                 traffic, so its scores describe the generator rather than the domain",
                Self::MIN_REVIEWED_EXAMPLES
            );
        }
        let mut reasons = Vec::new();
        if !self.beats_heuristic {
            reasons.push("does not beat the built-in detector".to_string());
        }
        if !self.beats_baseline {
            reasons.push("does not beat a linear model on the same features".to_string());
        }
        if !self.calibration_ok {
            reasons.push(
                "confidence is not calibrated enough to compare against the detector's".to_string(),
            );
        }
        format!("does not ship: {}", reasons.join("; "))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::corpus::{Example, Provenance};

    struct Always(FieldLabel, f64);
    impl Classifier for Always {
        fn name(&self) -> &str {
            "always"
        }
        fn classify(&self, _: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
            Some((self.0, self.1))
        }
    }

    struct Perfect;
    impl Classifier for Perfect {
        fn name(&self) -> &str {
            "perfect"
        }
        fn classify(&self, field: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
            FieldLabel::ALL
                .iter()
                .find(|label| label.name() == field.name)
                .map(|label| (*label, 0.95))
        }
    }

    struct Silent;
    impl Classifier for Silent {
        fn name(&self) -> &str {
            "silent"
        }
        fn classify(&self, _: &crate::Field<'_>) -> Option<(FieldLabel, f64)> {
            None
        }
    }

    fn corpus_of(labels: &[(FieldLabel, usize)]) -> Corpus {
        let mut examples = Vec::new();
        for (label, count) in labels {
            for _ in 0..*count {
                examples.push(Example::new(
                    label.name(),
                    vec!["v".to_string()],
                    *label,
                    Provenance::Generated,
                ));
            }
        }
        Corpus::new(examples)
    }

    #[test]
    fn answering_one_class_scores_badly_on_macro_f1_however_skewed_the_corpus() {
        // 95% opaque. Accuracy flatters the lazy answer; macro F1 does not.
        let corpus = corpus_of(&[(FieldLabel::Opaque, 95), (FieldLabel::Email, 5)]);
        let evaluation = evaluate(&Always(FieldLabel::Opaque, 0.9), &corpus);

        assert!(evaluation.accuracy() > 0.9);
        assert!(
            evaluation.macro_f1() < 0.55,
            "macro F1 was {:.3}, which would let a useless classifier through",
            evaluation.macro_f1()
        );
    }

    #[test]
    fn abstaining_counts_against_coverage_but_not_against_accuracy() {
        let corpus = corpus_of(&[(FieldLabel::Email, 10)]);
        let evaluation = evaluate(&Silent, &corpus);

        assert_eq!(evaluation.abstained, 10);
        assert_eq!(evaluation.coverage_accuracy(), 0.0);
        assert_eq!(evaluation.accuracy(), 0.0, "nothing was answered");
        assert_eq!(evaluation.macro_f1(), 0.0);
    }

    #[test]
    fn a_perfect_classifier_scores_one() {
        let corpus = corpus_of(&[
            (FieldLabel::Email, 4),
            (FieldLabel::Uuid, 4),
            (FieldLabel::Url, 4),
        ]);
        let evaluation = evaluate(&Perfect, &corpus);

        assert!((evaluation.macro_f1() - 1.0).abs() < 1e-9);
        assert!((evaluation.coverage_accuracy() - 1.0).abs() < 1e-9);
        assert!(evaluation.confusion.is_empty());
    }

    #[test]
    fn overconfidence_shows_up_as_calibration_error() {
        // Right a fifth of the time while claiming 0.95.
        let corpus = corpus_of(&[(FieldLabel::Email, 20), (FieldLabel::Uuid, 80)]);
        let evaluation = evaluate(&Always(FieldLabel::Email, 0.95), &corpus);

        assert!(
            evaluation.expected_calibration_error() > 0.5,
            "ECE was {:.3}",
            evaluation.expected_calibration_error()
        );
    }

    #[test]
    fn confusions_name_what_was_mistaken_for_what() {
        let corpus = corpus_of(&[(FieldLabel::Uuid, 7)]);
        let evaluation = evaluate(&Always(FieldLabel::Email, 0.5), &corpus);

        assert_eq!(
            evaluation.top_confusions(1),
            vec![((FieldLabel::Uuid, FieldLabel::Email), 7)]
        );
    }

    #[test]
    fn a_score_per_api_finds_the_family_an_average_would_hide() {
        // Right about every family but one. The mean says the model is fine; the
        // per-family table says which service is about to break.
        let mut examples = Vec::new();
        for _ in 0..100 {
            examples.push(
                Example::new(
                    "email",
                    vec!["v".to_string()],
                    FieldLabel::Email,
                    Provenance::Generated,
                )
                .from_source("good-family"),
            );
        }
        for _ in 0..20 {
            examples.push(
                Example::new(
                    "uuid",
                    vec!["v".to_string()],
                    FieldLabel::Uuid,
                    Provenance::Generated,
                )
                .from_source("bad-family"),
            );
        }
        let corpus = Corpus::new(examples);

        let scores = per_source(&Always(FieldLabel::Email, 0.9), &corpus);
        assert_eq!(scores.len(), 2);

        let bad = scores
            .iter()
            .find(|score| score.source == "bad-family")
            .unwrap();
        let good = scores
            .iter()
            .find(|score| score.source == "good-family")
            .unwrap();
        assert_eq!(bad.macro_f1, 0.0);
        assert!(good.macro_f1 > 0.99);
        assert!(
            evaluate(&Always(FieldLabel::Email, 0.9), &corpus).coverage_accuracy() > 0.8,
            "the overall number has to look good, or this test proves nothing"
        );
    }

    #[test]
    fn a_held_out_score_is_reported_against_the_detector_that_never_trained() {
        let corpus = Corpus::new(
            (0..30)
                .map(|_| {
                    Example::new(
                        FieldLabel::Uuid.name(),
                        vec!["v".to_string()],
                        FieldLabel::Uuid,
                        Provenance::Generated,
                    )
                    .from_source("unseen-family")
                })
                .collect(),
        );

        let strong = held_out(
            &Perfect,
            &Always(FieldLabel::Email, 0.5),
            &corpus,
            "unseen-family",
        );
        assert_eq!(strong.examples, 30);
        assert!(strong.beats_heuristic());

        let weak = held_out(
            &Always(FieldLabel::Email, 0.5),
            &Perfect,
            &corpus,
            "unseen-family",
        );
        assert!(!weak.beats_heuristic());

        let report = held_out_report(&[strong, weak]);
        assert!(report.contains("LOSES to the detector"), "{report}");
        assert!(report.contains("worst"), "{report}");
    }

    #[test]
    fn a_held_out_report_ranks_the_worst_family_first() {
        let score = |source: &str, candidate: f64| HeldOutScore {
            source: source.to_string(),
            examples: 100,
            candidate,
            heuristic: 0.5,
        };
        let report = held_out_report(&[score("fine", 0.95), score("broken", 0.20)]);
        let broken = report.find("broken").unwrap_or(usize::MAX);
        let fine = report.find("fine").unwrap_or(0);
        assert!(
            broken < fine,
            "the worst family has to be read first:\n{report}"
        );
    }

    /// A test split of reviewed examples, large enough to satisfy the gate.
    fn reviewed_corpus() -> Corpus {
        let examples = (0..ShipGate::MIN_REVIEWED_EXAMPLES + 10)
            .map(|n| {
                let label = if n % 2 == 0 {
                    FieldLabel::Email
                } else {
                    FieldLabel::Uuid
                };
                Example::new(
                    label.name(),
                    vec!["v".to_string()],
                    label,
                    Provenance::Reviewed,
                )
            })
            .collect();
        Corpus::new(examples)
    }

    #[test]
    fn the_gate_needs_every_bar_cleared() {
        let corpus = reviewed_corpus();
        let strong = Perfect;
        let weak = Always(FieldLabel::Email, 0.5);

        assert!(ShipGate::assess(&strong, &weak, &weak, &corpus).passed());
        assert!(!ShipGate::assess(&weak, &strong, &weak, &corpus).passed());
        assert!(!ShipGate::assess(&weak, &weak, &strong, &corpus).passed());

        let overconfident = Always(FieldLabel::Email, 1.0);
        let gate = ShipGate::assess(&overconfident, &weak, &weak, &corpus);
        assert!(!gate.calibration_ok);
        assert!(gate.explain().contains("calibrated"));
    }

    #[test]
    fn a_model_measured_only_on_generated_data_cannot_ship() {
        // The failure the previous attempt made: a model that has only ever met
        // its own generator scores beautifully and means nothing.
        let generated = corpus_of(&[(FieldLabel::Email, 100), (FieldLabel::Uuid, 100)]);

        assert!(
            evaluate(&Perfect, &generated).macro_f1() > 0.99,
            "the setup needs a model that looks perfect on the generator"
        );

        let gate = ShipGate::assess(
            &Perfect,
            &Always(FieldLabel::Email, 0.5),
            &Always(FieldLabel::Email, 0.5),
            &generated,
        );
        assert!(!gate.measured_on_real_data);
        assert!(
            !gate.passed(),
            "a perfect score on synthetic data is not a pass"
        );
        assert!(gate.explain().contains("generator"));
    }

    #[test]
    fn a_handful_of_reviewed_examples_is_not_enough() {
        let mut examples: Vec<Example> = corpus_of(&[(FieldLabel::Email, 100)]).examples;
        examples.push(Example::new(
            "email",
            vec!["a@b.com".to_string()],
            FieldLabel::Email,
            Provenance::Reviewed,
        ));
        let corpus = Corpus::new(examples);

        let gate = ShipGate::assess(
            &Perfect,
            &Always(FieldLabel::Uuid, 0.5),
            &Always(FieldLabel::Uuid, 0.5),
            &corpus,
        );
        assert!(!gate.measured_on_real_data);
    }

    #[test]
    fn generated_examples_cannot_carry_a_verdict_the_reviewed_ones_contradict() {
        // Enough reviewed examples to clear the count, swamped by generated ones.
        // A gate that scores the whole split reads the generator's 200 rows and
        // calls the model a winner; only the 60 reviewed rows say anything about
        // real traffic, and on those the model is beaten.
        let mut examples: Vec<Example> = corpus_of(&[(FieldLabel::Email, 200)]).examples;
        examples.extend((0..ShipGate::MIN_REVIEWED_EXAMPLES + 10).map(|_| {
            Example::new(
                FieldLabel::Uuid.name(),
                vec!["v".to_string()],
                FieldLabel::Uuid,
                Provenance::Reviewed,
            )
        }));
        let corpus = Corpus::new(examples);

        // `Always(Email)` is right on every generated row and wrong on every
        // reviewed one; `Always(Uuid)` is the reverse.
        let candidate = Always(FieldLabel::Email, 0.5);
        let rival = Always(FieldLabel::Uuid, 0.5);
        let gate = ShipGate::assess(&candidate, &rival, &rival, &corpus);

        assert!(
            gate.measured_on_real_data,
            "the count bar is cleared, which is what makes this the dangerous case"
        );
        assert!(
            evaluate(&candidate, &corpus).macro_f1() > evaluate(&rival, &corpus).macro_f1(),
            "the setup needs the candidate to win over the full split"
        );
        assert!(
            !gate.beats_heuristic,
            "the verdict must come from the reviewed rows, where the candidate loses"
        );
        assert!(!gate.passed());
    }
}
