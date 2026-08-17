//! Explaining the entity world a mocks directory builds.
//!
//! There is no `serve` here. A schema declares entities, not routes, so the
//! way to serve one is a mock with `serve:` and a `match.url` — which is also
//! the only place that can say the API lives behind `https://api.example.com`
//! rather than on localhost.

use std::path::PathBuf;

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
    }
}

fn report(world: &World, dir: &std::path::Path, mocks: usize) {
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
        crate::say!(
            "{}",
            ui::dim(&format!(
                "  {} → {} entities, served as {}",
                schema.path.display(),
                schema.entities.len(),
                schema.binding.protocol()
            ))
        );
    }

    // Merging is usually right and never right silently.
    for collision in world.collisions() {
        crate::say!(
            "{}",
            ui::warning(&format!("{collision} — they are merged into one entity"))
        );
    }

    let pending = world.pending_writes();
    if pending > 0 {
        crate::say!(
            "{}",
            ui::dim(&format!("{pending} write(s) laid over the seeded world"))
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
            .filter(|f| matches!(f.value, ValueSpec::Scalar(_) | ValueSpec::Enum(_)))
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
