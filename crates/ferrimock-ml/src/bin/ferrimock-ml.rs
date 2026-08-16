//! Train a field-type classifier, and say plainly whether it earned its place.
//!
//! ```text
//! ferrimock-ml train  [--corpus f.jsonl] [--per-label N] [--seed N] [--out model.json]
//! ferrimock-ml eval   --model model.json [--corpus f.jsonl]
//! ferrimock-ml gen    --out corpus.jsonl [--per-label N] [--seed N]
//! ferrimock-ml explain --model model.json [--label email]
//! ```
//!
//! `train` reports the candidate against the built-in detector and against a
//! linear model on the same features, on a test split neither of them was tuned
//! on, and refuses to write an artifact that loses. That refusal is the point:
//! the previous attempt at this had no bar to clear and so cleared none.

use ferrimock_ml::corpus::Provenance;
use ferrimock_ml::eval::ShipGate;
use ferrimock_ml::extract::{Candidate, ExtractOptions};
use ferrimock_ml::generator::Recipe;
use ferrimock_ml::generator::census::Census;
use ferrimock_ml::generator::dialect::ApiDialect;
use ferrimock_ml::linear::TrainingConfig;
use ferrimock_ml::merge::{
    MergeLabelOptions, MergeModel, MergeTrainingConfig, outcome_of, size_threshold_merges,
};
use ferrimock_ml::neural::{NeuralClassifier, NeuralConfig};
use ferrimock_ml::{
    Classifier, Corpus, FieldLabel, HeuristicClassifier, LinearClassifier, MergeExample,
    ModelArtifact, evaluate, held_out, held_out_report, per_source,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    let result = match command {
        "train" => train(&args),
        "eval" => run_eval(&args),
        "gen" => generate(&args),
        "census" => census(&args),
        "audit" => audit(&args),
        "explain" => explain(&args),
        "extract" => extract(&args),
        "promote" => promote(&args),
        "merge-label" => merge_label(&args),
        "merge-train" => merge_train(&args),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
ferrimock-ml -- field type classification

  train    fit a model and report whether it beats the detector and the baseline
  eval     score an existing model against a corpus
  gen      write a synthetic corpus
  census   count what a corpus actually covers, per label
  audit    find what the built-in detector gets wrong, and whether it is fixable
  explain  print what a model learned, per class
  extract  pull reviewable fields out of a recording (HAR or session)
  promote  turn a reviewed extract into a training corpus

  merge-label  measure which groups in a recording are safe to merge
  merge-train  fit a merge scorer and compare it against the size rule

options
  --corpus PATH     labelled examples as JSON Lines (default: generated)
  --count N         total examples to generate (default: --per-label x labels)
  --per-label N     examples per label when generating (default: 400)
  --seed N          seed for generation and splitting (default: 0)
  --epochs N        training passes (default: scaled to the corpus size)
  --out PATH        where to write the model or corpus
  --model PATH      model to load
  --label NAME      restrict `explain` to one label
  --recording PATH  HAR or recording session to extract fields from
  --mocks PATH      mock collection whose groups are measured
  --top N           how many rows an audit prints (default: 12)
  --holdout NAME    train without one API family, then score on it
  --holdout-scan    do that for every family in turn, and report the worst
  --neural          also fit a network on the same features and score it alongside
  --force           write the model even if it fails the ship gate

The generated corpus covers 17 families of API convention, 21 identifier shapes,
16 date formats and 20 locales. `census` reports what came out -- distinct names
and distinct value shapes per label -- so \"wide\" is a measurement rather than a
claim, and the labels it calls thin are the list of what to widen next.

`audit` is what the corpus and the model are for. Neither ships. The corpus knows
what every field really is, the detector is asked the same question, and its wrong
answers come back ranked, in the detector's own vocabulary, with concrete fields
to reproduce them. The model is scored on the same rows only to say which defects
are worth chasing: one a model reads right is a missing rule, one nothing reads
right is a field no reader could place.

`--holdout` is the nearest honest answer to \"will this work on an API nobody has
seen\": hold a whole family out of training, then score on it. `--holdout-scan`
does it for every family and reports the worst, which is the number that predicts
the next complaint.

`merge-label` needs no reviewer. It merges one group at a time, replays the
recording through the result, and labels the group by whether fidelity held --
so the label is the measured consequence of the merge rather than an opinion
about it. Slow by construction: one consolidation and one replay per group.

A model still cannot ship on generated data alone, however wide the generation.
`extract` writes one row per field found in a real recording, with the detector's
guess as a *suggestion* and the label left blank; fill the labels in, run
`promote`, and train against that.";

fn flag(args: &[String], name: &str) -> Option<String> {
    let position = args.iter().position(|arg| arg == name)?;
    args.get(position + 1).cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn number(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    match flag(args, name) {
        None => Ok(default),
        Some(raw) => raw
            .parse()
            .map_err(|_| format!("{name} expects a number, got `{raw}`")),
    }
}

/// The corpus a command was pointed at, or a generated one.
fn recipe(args: &[String]) -> Result<Recipe, String> {
    let seed = number(args, "--seed", 0)? as u64;
    if flag(args, "--count").is_some() {
        Ok(Recipe::new(number(args, "--count", 0)?, seed))
    } else {
        Ok(Recipe::balanced(number(args, "--per-label", 400)?, seed))
    }
}

fn load_corpus(args: &[String]) -> Result<Corpus, String> {
    if let Some(path) = flag(args, "--corpus") {
        Corpus::load(&path).map_err(|e| format!("could not read {path}: {e}"))
    } else {
        let recipe = recipe(args)?;
        eprintln!(
            "no --corpus given; generating {} examples across {} API families (seed {})",
            recipe.count,
            recipe.dialects.len(),
            recipe.seed
        );
        Ok(recipe.corpus())
    }
}

/// Training passes, scaled to the corpus unless the caller said otherwise.
///
/// A fixed epoch count is the wrong default at both ends: two hundred passes
/// over ten thousand examples is a few seconds, and over three million it is an
/// afternoon for no more signal. Total work is what should stay roughly fixed.
fn epochs_for(args: &[String], examples: usize) -> Result<usize, String> {
    if flag(args, "--epochs").is_some() {
        return number(args, "--epochs", 200);
    }
    let scaled = 4_000_000_usize.checked_div(examples.max(1)).unwrap_or(200);
    Ok(scaled.clamp(15, 200))
}

fn train(args: &[String]) -> Result<(), String> {
    if has_flag(args, "--holdout-scan") {
        return holdout_scan(args);
    }
    if let Some(family) = flag(args, "--holdout") {
        return holdout(args, &family);
    }

    let corpus = load_corpus(args)?;
    let seed = number(args, "--seed", 0)? as u64;
    let split = corpus.split(0.7, 0.15, seed);

    println!(
        "corpus {} examples ({} train / {} validation / {} test), {} labels",
        corpus.len(),
        split.train.len(),
        split.validation.len(),
        split.test.len(),
        corpus.label_counts().len()
    );

    let reviewed = corpus.reviewed_only().len();
    if reviewed == 0 {
        println!(
            "\nnote: every example is generated. A model measured only on synthetic data has \
             been measured against the generator, not against real traffic -- treat the numbers \
             below as a floor."
        );
    } else {
        println!("\n{reviewed} examples come from reviewed real traffic");
    }

    let epochs = epochs_for(args, split.train.len())?;
    let candidate = LinearClassifier::train(
        &split.train,
        TrainingConfig {
            seed,
            epochs,
            ..TrainingConfig::default()
        },
    );

    // The baseline is the same model fitted without class balancing, for a
    // quarter of the passes: the simplest thing anyone would try first on these
    // features.
    let baseline = LinearClassifier::train(
        &split.train,
        TrainingConfig {
            seed,
            balance_classes: false,
            epochs: (epochs / 4).max(10),
            ..TrainingConfig::default()
        },
    );
    let heuristic = HeuristicClassifier::new();

    // A network on the same features, so the question "would a network do
    // better" is answered rather than assumed. It reads the same split as
    // everything else.
    let neural = has_flag(args, "--neural").then(|| {
        eprintln!("fitting a network on the same features (this is the slow part)");
        NeuralClassifier::train(
            &split.train,
            &NeuralConfig {
                seed,
                ..NeuralConfig::default()
            },
        )
    });

    println!("\n--- validation ---");
    println!("{}", evaluate(&candidate, &split.validation).report());

    println!("--- test (held out) ---");
    let candidate_test = evaluate(&candidate, &split.test);
    let baseline_test = evaluate(&baseline, &split.test);
    let heuristic_test = evaluate(&heuristic, &split.test);

    println!("{}", heuristic_test.report());
    println!("{}", baseline_test.report());
    println!("{}", candidate_test.report());

    let neural_test = neural.as_ref().map(|model| evaluate(model, &split.test));
    if let Some(report) = neural_test.as_ref() {
        println!("{}", report.report());
    }

    println!("--- verdict ---");
    println!(
        "  macro-F1 over all {} test examples   detector {:.3}  baseline {:.3}  candidate {:.3}",
        split.test.len(),
        heuristic_test.macro_f1(),
        baseline_test.macro_f1(),
        candidate_test.macro_f1()
    );

    // The line above is the one that looks impressive; the line below is the one
    // the gate reads.
    let reviewed_test = split.test.reviewed_only();
    if reviewed_test.is_empty() {
        println!("  no reviewed examples in the test split, so the gate has nothing real to read");
    } else {
        println!(
            "  macro-F1 over {} reviewed examples   detector {:.3}  baseline {:.3}  candidate {:.3}",
            reviewed_test.len(),
            evaluate(&heuristic, &reviewed_test).macro_f1(),
            evaluate(&baseline, &reviewed_test).macro_f1(),
            evaluate(&candidate, &reviewed_test).macro_f1()
        );
    }

    if let Some(report) = neural_test.as_ref() {
        println!("  network on the same features   {:.3}", report.macro_f1());
        let verdict = if report.macro_f1() > candidate_test.macro_f1() {
            "the network beats the linear model on these features"
        } else {
            "the network does not beat a linear model on the same features, so the features \
             are the ceiling and more layers will not move it"
        };
        println!("  {verdict}");
    }

    // The overall macro-F1 is an average over families, and an average is
    // exactly what hides the family the model is useless on.
    let sources = per_source(&candidate, &split.test);
    if sources.len() > 1 {
        println!("\n--- per API family (all of these were trained on) ---");
        let mut ranked = sources;
        ranked.sort_by(|left, right| {
            left.macro_f1
                .partial_cmp(&right.macro_f1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for score in ranked.iter().take(6) {
            println!(
                "  {:<24} {:>6} examples   macro-F1 {:.3}",
                score.source, score.examples, score.macro_f1
            );
        }
        println!(
            "  (worst {} shown; run with --holdout-scan to score families the model never saw)",
            ranked.len().min(6)
        );
    }

    println!();
    let gate = ShipGate::assess(&candidate, &heuristic, &baseline, &split.test);
    println!("  {}", gate.explain());

    let Some(out) = flag(args, "--out") else {
        println!("\nno --out given; nothing written");
        return Ok(());
    };

    if !gate.passed() && !has_flag(args, "--force") {
        return Err(format!(
            "refusing to write {out}: {}. Re-run with --force to write it anyway.",
            gate.explain()
        ));
    }

    let note = format!(
        "trained on {} examples (seed {seed}); test macro-F1 {:.3} vs detector {:.3}",
        split.train.len(),
        candidate_test.macro_f1(),
        heuristic_test.macro_f1()
    );
    candidate
        .to_artifact()
        .with_note(note)
        .save(&out)
        .map_err(|e| format!("could not write {out}: {e}"))?;
    println!("\nwrote {out}");
    Ok(())
}

/// Train without one API family, then score on it.
///
/// Everything else measured here is measured on data drawn the same way the
/// training data was, and so says nothing about a convention the corpus does not
/// contain. This is the one measurement that does.
fn holdout(args: &[String], family: &str) -> Result<(), String> {
    let dialect = ApiDialect::from_name(family).ok_or_else(|| {
        let known: Vec<&str> = ApiDialect::ALL.iter().map(|d| d.name()).collect();
        format!(
            "no API family named `{family}`. Known: {}",
            known.join(", ")
        )
    })?;

    let score = train_without(args, dialect)?;
    println!("{}", held_out_report(&[score]));
    Ok(())
}

/// Do that for every family in turn.
fn holdout_scan(args: &[String]) -> Result<(), String> {
    println!(
        "training {} models, each without one API family, then scoring it on that family",
        ApiDialect::ALL.len()
    );

    let mut scores = Vec::with_capacity(ApiDialect::ALL.len());
    for dialect in ApiDialect::ALL {
        eprintln!("  holding out {}", dialect.name());
        scores.push(train_without(args, dialect)?);
    }

    println!("\n{}", held_out_report(&scores));

    let lost: Vec<&str> = scores
        .iter()
        .filter(|score| !score.beats_heuristic())
        .map(|score| score.source.as_str())
        .collect();
    if lost.is_empty() {
        println!("the model beats the built-in detector on every family it was never trained on");
    } else {
        println!(
            "the model loses to the built-in detector on: {}",
            lost.join(", ")
        );
    }
    Ok(())
}

/// Fit on every family but one, and score on the one left out.
fn train_without(
    args: &[String],
    held_out_family: ApiDialect,
) -> Result<ferrimock_ml::HeldOutScore, String> {
    let seed = number(args, "--seed", 0)? as u64;
    let plan = recipe(args)?;

    let training = plan.clone().without(held_out_family).corpus();
    // A quarter as many rows to score on: the question is how the model does on
    // the family, and a smaller sample answers it without doubling the run.
    let unseen = Recipe::new(plan.count / 4, seed ^ 0xA5A5)
        .only(vec![held_out_family])
        .corpus();

    let epochs = epochs_for(args, training.len())?;
    let model = LinearClassifier::train(
        &training,
        TrainingConfig {
            seed,
            epochs,
            ..TrainingConfig::default()
        },
    );

    Ok(held_out(
        &model,
        &HeuristicClassifier::new(),
        &unseen,
        held_out_family.name(),
    ))
}

/// Count what a corpus covers, rather than how many rows it has.
fn census(args: &[String]) -> Result<(), String> {
    let census = if let Some(path) = flag(args, "--corpus") {
        let corpus = Corpus::load(&path).map_err(|e| format!("could not read {path}: {e}"))?;
        Census::of(&corpus)
    } else {
        let plan = recipe(args)?;
        plan.census(plan.count)
    };

    println!("{}", census.report());

    let thin = census.thin(25, 15);
    if thin.is_empty() {
        println!("every label is covered by at least 25 names and 15 value shapes");
    } else {
        println!("thinly covered, and worth widening first:");
        for (label, coverage) in thin {
            println!(
                "  {:<20} {} names, {} value shapes",
                label.name(),
                coverage.distinct_names,
                coverage.distinct_shapes
            );
        }
    }
    Ok(())
}

/// Find what the built-in detector gets wrong, and say which of it is fixable.
fn audit(args: &[String]) -> Result<(), String> {
    let corpus = load_corpus(args)?;
    let seed = number(args, "--seed", 0)? as u64;
    let top = number(args, "--top", 12)?;

    // Fitted on the same corpus it audits, on purpose. The question is not how
    // the model generalises -- it never ships -- but how much signal these rows
    // contain at all, which is the ceiling the detector is being measured
    // against.
    let epochs = epochs_for(args, corpus.len())?;
    eprintln!(
        "fitting a reference model over {} fields to measure the detector against",
        corpus.len()
    );
    let reference = LinearClassifier::train(
        &corpus,
        TrainingConfig {
            seed,
            epochs,
            ..TrainingConfig::default()
        },
    );

    let audit = ferrimock_ml::audit::audit(&corpus, &reference, 4);
    println!("{}", ferrimock_ml::audit::report(&audit, &reference, top));

    let recoverable: usize = audit
        .defects
        .iter()
        .filter(|defect| defect.recoverable >= 0.8)
        .map(|defect| defect.count)
        .sum();
    let ambiguous: usize = audit
        .defects
        .iter()
        .filter(|defect| defect.recoverable < 0.4)
        .map(|defect| defect.count)
        .sum();
    let total: usize = audit.defects.iter().map(|defect| defect.count).sum();

    println!(
        "{total} of {} fields are answered wrongly: {recoverable} of them by a rule that is \
         missing, {ambiguous} by nothing anything could have read.",
        audit.examples
    );
    println!(
        "Fixing the recoverable ones in crates/ferrimock/src/type_detector is the work; \
         the model is only how they were found."
    );
    Ok(())
}

fn run_eval(args: &[String]) -> Result<(), String> {
    let path = flag(args, "--model").ok_or("eval needs --model PATH")?;
    let artifact = ModelArtifact::load(&path).map_err(|e| e.to_string())?;
    let corpus = load_corpus(args)?;

    if !artifact.note.is_empty() {
        println!("model: {}", artifact.note);
    }
    println!("{}", evaluate(&artifact.model, &corpus).report());
    println!(
        "{}",
        evaluate(&HeuristicClassifier::new(), &corpus).report()
    );
    Ok(())
}

fn generate(args: &[String]) -> Result<(), String> {
    let out = flag(args, "--out").ok_or("gen needs --out PATH")?;
    let plan = recipe(args)?;

    // Streamed rather than collected: a corpus of millions is larger than it is
    // worth holding in memory, and an indexable recipe never has to.
    plan.write_jsonl(&out)
        .map_err(|e| format!("could not write {out}: {e}"))?;

    println!(
        "wrote {} examples to {out}, drawn from {} API families (seed {}), all {:?}",
        plan.count,
        plan.dialects.len(),
        plan.seed,
        Provenance::Generated
    );
    println!("  ferrimock-ml census --corpus {out}   to see what it actually covers");
    Ok(())
}

fn explain(args: &[String]) -> Result<(), String> {
    let path = flag(args, "--model").ok_or("explain needs --model PATH")?;
    let artifact = ModelArtifact::load(&path).map_err(|e| e.to_string())?;

    let labels: Vec<FieldLabel> = match flag(args, "--label") {
        Some(name) => vec![
            *FieldLabel::ALL
                .iter()
                .find(|label| label.name() == name)
                .ok_or_else(|| format!("no label named `{name}`"))?,
        ],
        None => FieldLabel::ALL.to_vec(),
    };

    for label in labels {
        println!("{}:", label.name());
        for (feature, weight) in artifact.model.explain(label, 6) {
            println!("  {weight:+.3}  {feature}");
        }
        println!();
    }
    Ok(())
}

fn extract(args: &[String]) -> Result<(), String> {
    let recording = flag(args, "--recording").ok_or("extract needs --recording PATH")?;
    let out = flag(args, "--out").ok_or("extract needs --out PATH")?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start a runtime: {e}"))?;
    let interactions = runtime
        .block_on(ferrimock::recorder::load_interactions(&recording))
        .map_err(|e| format!("could not read {recording}: {e}"))?;

    let candidates =
        ferrimock_ml::extract::from_interactions(&interactions, &ExtractOptions::default());

    let mut text = String::new();
    for candidate in &candidates {
        text.push_str(
            &serde_json::to_string(candidate).map_err(|e| format!("could not serialise: {e}"))?,
        );
        text.push('\n');
    }
    std::fs::write(&out, text).map_err(|e| format!("could not write {out}: {e}"))?;

    let suggested = candidates.iter().filter(|c| c.suggestion.is_some()).count();
    println!(
        "wrote {} fields from {} interactions to {out}",
        candidates.len(),
        interactions.len()
    );
    println!("  {suggested} carry a suggestion; none carry a label");
    println!();
    println!("Fill in `label` on the rows you are sure of, then:");
    println!("  ferrimock-ml promote --recording {out} --out corpus.jsonl");
    Ok(())
}

fn promote(args: &[String]) -> Result<(), String> {
    let reviewed = flag(args, "--recording").ok_or("promote needs --recording PATH")?;
    let out = flag(args, "--out").ok_or("promote needs --out PATH")?;

    let text = std::fs::read_to_string(&reviewed)
        .map_err(|e| format!("could not read {reviewed}: {e}"))?;
    let candidates: Vec<Candidate> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{reviewed} is not an extract file: {e}"))?;

    let total = candidates.len();
    let corpus = ferrimock_ml::extract::reviewed_corpus(candidates);
    corpus
        .save(&out)
        .map_err(|e| format!("could not write {out}: {e}"))?;

    println!(
        "{} of {total} fields were labelled; wrote {out}",
        corpus.len()
    );
    if corpus.len() < ShipGate::MIN_REVIEWED_EXAMPLES {
        println!(
            "  note: the ship gate needs at least {} reviewed examples before a model \
             trained on them can be trusted",
            ShipGate::MIN_REVIEWED_EXAMPLES
        );
    }
    Ok(())
}

fn merge_label(args: &[String]) -> Result<(), String> {
    let recording = flag(args, "--recording").ok_or("merge-label needs --recording PATH")?;
    let out = flag(args, "--out").ok_or("merge-label needs --out PATH")?;
    let seed = number(args, "--seed", 0)? as u64;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start a runtime: {e}"))?;

    let interactions = runtime
        .block_on(ferrimock::recorder::load_interactions(&recording))
        .map_err(|e| format!("could not read {recording}: {e}"))?;

    let profile = ferrimock::profile::default_profile();
    let fidelity = ferrimock::consolidator::FidelityOptions {
        base_dir: std::path::Path::new(&recording)
            .parent()
            .map(std::path::Path::to_path_buf),
        // Nothing else serves mocks in this process, and pagination counters
        // left by one group's replay would skew the next one's.
        reset_persistence: true,
        ..ferrimock::consolidator::FidelityOptions::default()
    };

    let options = MergeLabelOptions {
        seed,
        ..MergeLabelOptions::default()
    };
    println!(
        "{} interactions; {:.0}% held out of the mocks to judge the merges on",
        interactions.len(),
        options.holdout_ratio * 100.0
    );

    let examples = runtime.block_on(ferrimock_ml::merge::label_groups(
        &interactions,
        &profile,
        &fidelity,
        &options,
    ))?;

    let unsafe_count = examples.iter().filter(|example| !example.safe).count();
    MergeExample::save(&examples, &out)?;

    println!(
        "measured {} groups; {} merge safely, {unsafe_count} do not; wrote {out}",
        examples.len(),
        examples.len() - unsafe_count
    );
    for example in examples.iter().filter(|example| !example.safe) {
        println!(
            "  unsafe  behavioural {:+.3}  values {:+.3}",
            example.behavioral_delta, example.value_delta
        );
        for request in &example.requests {
            println!("            {request}");
        }
    }

    Ok(())
}

fn merge_train(args: &[String]) -> Result<(), String> {
    let corpus = flag(args, "--corpus").ok_or("merge-train needs --corpus PATH")?;
    let seed = number(args, "--seed", 0)? as u64;

    let mut examples = MergeExample::load(&corpus)?;
    if examples.is_empty() {
        return Err(format!("{corpus} holds no measured groups"));
    }

    // Held out by seeded shuffle rather than by file order, which follows the
    // recording and would put whole endpoints on one side of the split.
    shuffle_examples(&mut examples, seed);
    let split = examples.len().saturating_mul(3) / 4;
    let (train, test) = examples.split_at(split.max(1).min(examples.len() - 1));

    let model = MergeModel::train(
        train,
        &MergeTrainingConfig {
            seed,
            ..MergeTrainingConfig::default()
        },
    )?;

    println!(
        "measured {} groups ({} train / {} held out)",
        examples.len(),
        train.len(),
        test.len()
    );
    println!();

    let rule = outcome_of(test, size_threshold_merges);
    let learned = outcome_of(test, |example| model.probability(&example.features) >= 0.5);

    for (name, outcome) in [("size rule", rule), ("learned", learned)] {
        println!("{name}:");
        println!(
            "  unsafe merges {} of {}  (caught {:.1}%)",
            outcome.merged_unsafely,
            outcome.merged_unsafely + outcome.refused_unsafely,
            outcome.unsafe_caught() * 100.0
        );
        println!(
            "  safe merges   {} of {}  (took {:.1}%)",
            outcome.merged_safely,
            outcome.merged_safely + outcome.refused_safely,
            outcome.safe_merged() * 100.0
        );
    }
    println!();

    println!("what the model leans on:");
    for (feature, weight) in model.explain().iter().take(6) {
        println!("  {weight:+.3}  {feature}");
    }
    println!();

    // Merging something unsafe breaks a mock; refusing something safe only
    // leaves the collection larger. A model that trades the first for the second
    // is not an improvement, however much better its accuracy looks.
    if learned.unsafe_caught() < rule.unsafe_caught() {
        println!(
            "does not ship: catches less of what is unsafe than the rule it replaces \
             ({:.1}% against {:.1}%)",
            learned.unsafe_caught() * 100.0,
            rule.unsafe_caught() * 100.0
        );
    } else if (learned.unsafe_caught() - rule.unsafe_caught()).abs() < f64::EPSILON
        && learned.safe_merged() <= rule.safe_merged()
    {
        println!("does not ship: catches no more and merges no more than the size rule");
    } else {
        println!("beats the size rule on the held-out groups");
    }

    if test.len() < 20 {
        println!(
            "  note: {} held-out groups is too few to separate these rules; measure more \
             recordings before acting on the comparison",
            test.len()
        );
    }

    let Some(out) = flag(args, "--out") else {
        println!("\nno --out given; nothing written");
        return Ok(());
    };
    let text =
        serde_json::to_string_pretty(&model).map_err(|e| format!("could not serialise: {e}"))?;
    std::fs::write(&out, text).map_err(|e| format!("could not write {out}: {e}"))?;
    println!("\nwrote {out}");
    Ok(())
}

/// Deterministic shuffle, so a seed reproduces a split exactly.
fn shuffle_examples(examples: &mut [MergeExample], seed: u64) {
    let mut state = seed | 1;
    for index in (1..examples.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_possible_truncation)] // modulo keeps this in range
        let swap = (state % (index as u64 + 1)) as usize;
        examples.swap(index, swap);
    }
}

/// Keeps the binary honest about what it depends on.
#[allow(dead_code)]
fn assert_classifier_object_safe(classifier: &dyn Classifier) -> &str {
    classifier.name()
}
