//! Explaining the entity world a mocks directory builds.
//!
//! There is no `serve` here. A schema declares entities, not routes, so the
//! way to serve one is a mock with `serve:` and a `match.url` — which is also
//! the only place that can say the API lives behind `https://api.example.com`
//! rather than on localhost.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Subcommand};
use ferrimock::core::World;
use ferrimock::core::world::model::{Cardinality, ValueSpec};
use ferrimock::engine::MockRegistry;

use super::ui;

#[derive(Args)]
pub struct WorldCommand {
    #[command(subcommand)]
    pub action: WorldAction,
}

#[derive(Subcommand)]
pub enum WorldAction {
    /// Print the entity world a mocks directory builds, and what it cost
    Explain {
        /// Directory of mocks and schemas (defaults to the configured one)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Show every entity's value fields, not just its relations
        #[arg(short, long)]
        verbose: bool,
    },

    /// Lint the generated world for the things that give a mock away
    Doctor {
        /// Directory of mocks and schemas (defaults to the configured one)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Exit non-zero when anything at all is reported, not just a defect
        #[arg(long)]
        strict: bool,
    },
}

pub async fn execute(command: WorldCommand) -> anyhow::Result<()> {
    match command.action {
        WorldAction::Explain { dir, verbose } => {
            let registry = MockRegistry::new();
            let dir = dir.unwrap_or_else(|| PathBuf::from(crate::config::mocks_dir()));

            let mocks = registry
                .load_from_directory(&dir)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            let world = registry.world();
            report(world, &dir, mocks);
            explain(world, verbose);
            Ok(())
        }
        WorldAction::Doctor { dir, strict } => {
            let registry = MockRegistry::new();
            let dir = dir.unwrap_or_else(|| PathBuf::from(crate::config::mocks_dir()));

            registry
                .load_from_directory(&dir)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            let world = registry.world();
            if world.is_empty() {
                crate::say!(
                    "{}",
                    ui::warning(&format!(
                        "no schemas in {} — nothing to lint",
                        dir.display()
                    ))
                );
                return Ok(());
            }
            diagnose(&world.store(), strict)
        }
    }
}

/// Print what a client could tell about this world, worst first.
///
/// Every proposal against the engine either moves a number here or does not
/// ship, so the report leads with the counts rather than with the prose.
fn diagnose(
    store: &ferrimock::core::world::store::EntityStore,
    strict: bool,
) -> anyhow::Result<()> {
    use ferrimock::core::world::doctor::{self, Severity};

    let report = doctor::examine(store);

    crate::say!("{}", ui::header("World doctor"));
    crate::say!();
    crate::say!(
        "{}",
        ui::info(&format!(
            "{} record(s) read · {} defect(s) · {} tell(s) · {} check(s) the world is too small for",
            report.sampled,
            report.broken(),
            report.findings.len() - report.broken(),
            report.unmeasured.len()
        ))
    );

    for severity in [Severity::Broken, Severity::Tell] {
        let group: Vec<_> = report
            .findings
            .iter()
            .filter(|finding| finding.check.severity() == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        crate::say!();
        crate::say!(
            "{}",
            ui::header(match severity {
                Severity::Broken => "Defects",
                Severity::Tell => "Tells",
            })
        );
        for finding in group {
            let line = format!(
                "{} {} — {}",
                finding.check.name(),
                finding.subject,
                finding.check.tell()
            );
            crate::say!(
                "{}",
                match severity {
                    Severity::Broken => ui::error(&line),
                    Severity::Tell => ui::warning(&line),
                }
            );
            crate::say!("{}", ui::dim(&format!("    {}", finding.measured)));
        }
    }

    if !report.unmeasured.is_empty() {
        crate::say!();
        crate::say!("{}", ui::header("Not measurable in this world"));
        for item in &report.unmeasured {
            crate::say!(
                "{}",
                ui::dim(&format!(
                    "  {} {} — needs {}",
                    item.check.name(),
                    item.subject,
                    item.needs
                ))
            );
        }
    }

    if report.broken() > 0 || (strict && !report.is_clean()) {
        anyhow::bail!("world doctor reported {} finding(s)", report.findings.len());
    }
    Ok(())
}

fn report(world: &Arc<World>, dir: &std::path::Path, mocks: usize) {
    crate::say!("{}", ui::header("World"));
    crate::say!();

    if world.is_empty() {
        crate::say!(
            "{}",
            ui::warning(&format!(
                "no schemas in {} — a `.graphql` beside the mocks, or a `world.schemas` \
                 entry in a collection, is what puts entities here",
                dir.display()
            ))
        );
        return;
    }

    crate::say!(
        "{}",
        ui::info(&format!(
            "{} entities from {} schema(s), seed {} · {mocks} mock(s) loaded",
            world.entities().len(),
            world.schemas().len(),
            world.seed()
        ))
    );

    for schema in world.schemas() {
        let endpoints = schema
            .binding
            .endpoints()
            .map_or_else(String::new, |count| format!(", {count} endpoint(s)"));
        crate::say!(
            "{}",
            ui::dim(&format!(
                "  {} → {} entities{endpoints}, served as {}",
                schema.path.display(),
                schema.entities.len(),
                schema.binding.protocol()
            ))
        );
        report_coverage(world, &schema);
    }

    // Merging is usually right and never right silently.
    for collision in world.collisions() {
        crate::say!(
            "{}",
            ui::warning(&format!(
                "{collision} — their fields are merged into one entity"
            ))
        );
    }

    // A rule that did not apply is nearly always a typo or a renamed field, and
    // the payload it silently fails to change looks almost right.
    for rejected in world.rejected_overrides() {
        crate::say!("{}", ui::warning(&format!("{rejected}")));
    }

    let pending = world.pending_writes();
    if pending > 0 {
        crate::say!(
            "{}",
            ui::dim(&format!("{pending} write(s) laid over the seeded world"))
        );
    }
}

/// How much of a document is answered from the store, and how much is
/// invented.
///
/// The honest way to present a generated backend is to lead with that number.
/// A mock that makes up half an API must not look like one that does not.
fn report_coverage(world: &Arc<World>, schema: &ferrimock::core::world::LoadedSchema) {
    use ferrimock::core::world::Binding;
    use ferrimock::spec::bind::rest::RestBackend;

    let Binding::OpenApi(table) = &schema.binding else {
        return;
    };

    let backend = RestBackend::build(table, world);
    let coverage = backend.coverage();
    let unclassified = coverage.unclassified().len();
    if unclassified == 0 {
        return;
    }

    let line = format!(
        "    {:.0}% answered from the world; {unclassified} operation(s) answer from their \
         declared shape alone",
        coverage.ratio() * 100.0
    );
    crate::say!("{}", ui::warning(&line));
    for id in coverage.unclassified().iter().take(5) {
        crate::say!("{}", ui::dim(&format!("      {id}")));
    }
    if unclassified > 5 {
        crate::say!(
            "{}",
            ui::dim(&format!("      … and {} more", unclassified - 5))
        );
    }
}

fn explain(world: &World, verbose: bool) {
    if world.is_empty() {
        return;
    }

    let graph = world.graph();
    crate::say!();
    crate::say!("{}", ui::header("Entities"));

    for entity in graph.entities() {
        let key = entity
            .key
            .iter()
            .map(|part| part.field.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        crate::say!();
        crate::say!(
            "{} {}",
            entity.name.as_str(),
            ui::dim(&format!(
                "key {key} · {} instance(s) · {}",
                world.count(entity.name.as_str()),
                entity.provenance.rule
            ))
        );

        for (field, relation) in entity.relations() {
            let arity = match relation.cardinality {
                Cardinality::One => "one",
                Cardinality::Many => "many",
            };
            crate::say!(
                "  {} {} {}",
                ui::dim("→"),
                format_args!("{}: {arity} {}", field.name, relation.target),
                ui::dim(&format!(
                    "[{} {:.0}%]",
                    relation.provenance.rule,
                    relation.confidence.value() * 100.0
                ))
            );
        }

        let scalars: Vec<&str> = entity
            .value_fields()
            .filter(|f| {
                matches!(
                    f.value,
                    ValueSpec::Scalar(_) | ValueSpec::Enum(_) | ValueSpec::Lifecycle(_)
                )
            })
            .map(|f| f.name.as_str())
            .collect();
        if !scalars.is_empty() {
            if verbose {
                crate::say!("  {} {}", ui::dim("·"), ui::dim(&scalars.join(", ")));
            } else {
                crate::say!("  {} {} value field(s)", ui::dim("·"), scalars.len());
            }
        }
    }

    // Only an unsatisfiable cut is worth a reader's attention: a to-many edge
    // can always be empty, so breaking one costs nothing.
    let order = graph.seed_order();
    let unsatisfiable: Vec<_> = order
        .broken_cycles
        .iter()
        .filter(|cut| cut.is_unsatisfiable())
        .collect();
    if !unsatisfiable.is_empty() {
        crate::say!();
        crate::say!("{}", ui::header("Unsatisfiable relations"));
        for cut in unsatisfiable {
            crate::say!(
                "{}",
                ui::warning(&format!(
                    "{}.{} → {} is non-nullable inside a cycle; no finite world can fill it",
                    cut.from, cut.field, cut.to
                ))
            );
        }
    }
}
