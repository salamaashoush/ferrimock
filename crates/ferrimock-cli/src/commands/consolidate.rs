//! Consolidate and optimize mock collections

use super::ui;
use anyhow::Context;
use ferrimock::config::MockCollectionConfig;
use ferrimock::consolidator::{
    ConsolidatorOptions, FidelityOptions, FidelityReport, MockConsolidator,
};

pub struct ConsolidateArgs {
    pub input: String,
    pub output: String,
    pub format: String,
    pub min_pattern: usize,
    pub enable_templates: bool,
    pub generalize: bool,
    pub verify: Option<String>,
    pub fail_under: Option<f64>,
    pub verbose: bool,
}

pub async fn consolidate_mocks(args: ConsolidateArgs) -> anyhow::Result<()> {
    let ConsolidateArgs {
        input,
        output,
        format,
        min_pattern,
        enable_templates,
        generalize,
        verify,
        fail_under,
        verbose,
    } = args;

    crate::say!("{}", ui::action("Consolidating mock collection"));
    crate::say!();
    crate::say!("{}", ui::kv("Input", &ui::path(&input)));
    crate::say!("{}", ui::kv("Output", &ui::path(&output)));
    crate::say!("{}", ui::kv("Format", &format));
    if let Some(recording) = verify.as_ref() {
        crate::say!("{}", ui::kv("Verify against", &ui::path(recording)));
    }
    crate::say!();

    if verbose {
        crate::say!("{}", ui::header("Optimization Settings"));
        crate::say!(
            "{}",
            ui::kv("  Min pattern threshold", &ui::number(min_pattern))
        );
        crate::say!("{}", ui::kv("  Pagination detection", "automatic"));
        crate::say!("{}", ui::kv("  ID pattern detection", "automatic"));
        crate::say!(
            "{}",
            ui::kv("  Template extraction", &enable_templates.to_string())
        );
        crate::say!(
            "{}",
            ui::kv("  Generalize lone recordings", &generalize.to_string())
        );
        crate::say!();
    }

    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates,
        min_pattern_threshold: min_pattern,
        generalize,
        enable_stateful_pagination: true,
        pagination_storage_key_template: "api.{path}.total".to_string(),
        ..ConsolidatorOptions::default()
    };

    let mut consolidator = MockConsolidator::with_options(options);

    let spinner = ui::spinner("Loading and analyzing mocks...");
    let collection = MockCollectionConfig::from_file(std::path::PathBuf::from(&input))
        .await
        .context("Failed to load mock collection")?;

    let (consolidated, report) = match verify.as_ref() {
        None => (
            consolidator
                .consolidate(collection)
                .context("Failed to consolidate mocks")?,
            None,
        ),
        Some(recording) => {
            let interactions = ferrimock::recorder::load_interactions(recording)
                .await
                .with_context(|| format!("Failed to load recording {recording}"))?;
            let fidelity = FidelityOptions {
                base_dir: std::path::Path::new(&input)
                    .parent()
                    .map(std::path::Path::to_path_buf),
                // Nothing else is serving mocks in this process, so pagination
                // counters left over from a previous run would only skew the
                // comparison.
                reset_persistence: true,
                ..FidelityOptions::default()
            };
            let (consolidated, report) = consolidator
                .consolidate_verified(&interactions, collection, &fidelity)
                .await
                .context("Failed to consolidate and verify mocks")?;
            (consolidated, Some(report))
        }
    };
    spinner.finish_and_clear();

    crate::say!();
    print_stats(&consolidator);

    let spinner = ui::spinner("Saving consolidated mocks...");
    let content = match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(&consolidated)?,
        "yaml" | "yml" => {
            serde_yaml_ng::to_string(&consolidated).context("YAML serialization error")?
        }
        _ => {
            anyhow::bail!("Invalid format: {format}. Use 'json' or 'yaml'");
        }
    };
    tokio::fs::write(&output, content).await?;
    spinner.finish_and_clear();

    crate::say!("{}", ui::success("Successfully consolidated mocks!"));
    crate::say!();
    crate::say!("{}", ui::kv("Output file", &ui::path(&output)));

    if let (Ok(input_metadata), Ok(output_metadata)) =
        (std::fs::metadata(&input), std::fs::metadata(&output))
    {
        let input_size = input_metadata.len();
        let output_size = output_metadata.len();
        #[allow(clippy::cast_precision_loss)]
        let savings = (1.0 - (output_size as f64 / input_size as f64)) * 100.0;

        crate::say!("{}", ui::kv("Input size", &ui::format_bytes(input_size)));
        crate::say!("{}", ui::kv("Output size", &ui::format_bytes(output_size)));
        crate::say!("{}", ui::kv("Space saved", &format!("{savings:.1}%")));
    }

    if let Some(report) = report {
        crate::say!();
        print_fidelity(&report, verbose);

        if let Some(threshold) = fail_under
            && !report.passes(threshold)
        {
            anyhow::bail!(
                "Behavioural fidelity {:.1}% is below the required {:.1}%",
                report.score.behavioral_ratio() * 100.0,
                threshold * 100.0
            );
        }
    }

    Ok(())
}

fn print_stats(consolidator: &MockConsolidator) {
    let stats = consolidator.stats();
    crate::say!("{}", ui::header("Consolidation"));
    crate::say!(
        "{}",
        ui::kv("  Original mocks", &ui::number(stats.original_count))
    );
    crate::say!(
        "{}",
        ui::kv(
            "  Consolidated mocks",
            &ui::number(stats.consolidated_count)
        )
    );
    crate::say!(
        "{}",
        ui::kv(
            "  Reduction",
            &format!("{:.1}%", stats.reduction_ratio * 100.0)
        )
    );
    crate::say!(
        "{}",
        ui::kv("  Patterns detected", &ui::number(stats.patterns_detected))
    );
    crate::say!(
        "{}",
        ui::kv(
            "  Duplicates removed",
            &ui::number(stats.duplicates_removed)
        )
    );
    crate::say!(
        "{}",
        ui::kv("  Templates created", &ui::number(stats.templates_created))
    );
}

/// Render the replay report.
///
/// The reduction number above is meaningless on its own -- collapsing every mock
/// into one would score 99% -- so this is the half that says whether the
/// collection still behaves like the traffic it came from.
fn print_fidelity(report: &FidelityReport, verbose: bool) {
    crate::say!(
        "{}",
        ui::header("Fidelity (replayed against the recording)")
    );

    let level = |label: &str, part: usize, total: usize, ratio: f64| {
        crate::say!(
            "{}",
            ui::kv(label, &format!("{part}/{total} ({:.1}%)", ratio * 100.0))
        );
    };

    let total = report.score.total;
    level(
        "  Matched",
        report.score.matched,
        total,
        report.score.matched_ratio(),
    );
    level(
        "  Right lineage",
        report.score.no_cross_talk,
        total,
        report.score.no_cross_talk_ratio(),
    );
    level(
        "  Status exact",
        report.score.status_exact,
        total,
        report.score.status_exact_ratio(),
    );
    level(
        "  Shape equal",
        report.score.shape_equal,
        total,
        report.score.shape_equal_ratio(),
    );
    level(
        "  Constants held",
        report.score.constants_held,
        total,
        report.score.constants_held_ratio(),
    );
    // The level templating deliberately trades away, and the one a request-echo
    // wins back. Without it on screen, a template that answers about the thing
    // that was asked for looks identical to one that invents a value.
    level(
        "  Values equal",
        report.score.value_equal,
        total,
        report.score.value_equal_ratio(),
    );
    // Whole-response equality is all-or-nothing, so one field answering
    // correctly moves nothing. Per-leaf agreement is where that shows.
    level(
        "  Leaf values equal",
        report.score.leaves_equal,
        report.score.leaves,
        report.score.leaves_equal_ratio(),
    );
    crate::say!();
    crate::say!(
        "{}",
        ui::kv(
            "  Behavioural",
            &format!("{:.1}%", report.score.behavioral_ratio() * 100.0)
        )
    );
    crate::say!(
        "{}",
        ui::kv(
            "  Cost vs unconsolidated",
            &format!("{:+.1} pts", report.behavioral_delta() * 100.0)
        )
    );

    if report.baseline.behavioral < report.baseline.total {
        crate::say!(
            "{}",
            ui::warning(&format!(
                "{} of {} requests already replayed wrong before consolidation; \
                 those are recording or matching gaps, and consolidation cannot fix them",
                report.baseline.total - report.baseline.behavioral,
                report.baseline.total
            ))
        );
    }

    let sections: [(&str, usize); 5] = [
        ("unmatched", report.unmatched.len()),
        ("cross-talk", report.cross_talk.len()),
        ("status", report.status_mismatch.len()),
        ("shape", report.shape_mismatch.len()),
        ("constant drift", report.constant_drift.len()),
    ];
    if sections.iter().any(|(_, count)| *count > 0) {
        crate::say!();
        crate::say!("{}", ui::header("Divergences"));
    }

    for reference in &report.unmatched {
        crate::say!(
            "{}",
            ui::dim(&format!(
                "  unmatched  {} {}",
                reference.method, reference.target
            ))
        );
    }
    for cross in &report.cross_talk {
        crate::say!(
            "{}",
            ui::dim(&format!(
                "  cross-talk {} {} -> {} (expected lineage {})",
                cross.interaction.method,
                cross.interaction.target,
                cross.matched_mock,
                cross.expected_origin
            ))
        );
    }
    for (label, divergences) in [
        ("status", &report.status_mismatch),
        ("shape", &report.shape_mismatch),
        ("constant", &report.constant_drift),
        ("render", &report.render_errors),
    ] {
        for divergence in divergences {
            crate::say!(
                "{}",
                ui::dim(&format!(
                    "  {label:<10} {} {} :: {}",
                    divergence.interaction.method, divergence.interaction.target, divergence.detail
                ))
            );
        }
    }

    if report.examples_capped && !verbose {
        crate::say!(
            "{}",
            ui::dim("  (example lists truncated; the counts above are complete)")
        );
    }
}
