//! Serving and explaining a spec-derived backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::{Args, Subcommand};
use ferrimock::spec::bind::graphql::GraphQLBackend;
use ferrimock::spec::infer::graphql::{parse_sdl, parse_sdl_lenient, to_entity_graph};
use ferrimock::spec::model::{Cardinality, EntityGraph, ValueSpec};
use ferrimock::spec::store::{EntityStore, StoreConfig};

use super::ui;

#[derive(Args)]
pub struct SpecCommand {
    #[command(subcommand)]
    pub action: SpecAction,
}

#[derive(Subcommand)]
pub enum SpecAction {
    /// Serve a spec as a stateful backend
    Serve {
        /// GraphQL schema file (.graphql / .gql)
        spec: PathBuf,

        #[arg(short, long, default_value = "3000")]
        port: u16,

        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Endpoint the GraphQL backend answers on
        #[arg(long, default_value = "/graphql")]
        endpoint: String,

        /// Seed for the generated world; the same seed rebuilds it exactly
        #[arg(long, default_value = "0")]
        seed: u64,

        /// Instances per entity
        #[arg(long, default_value = "12")]
        count: usize,

        /// Per-entity instance counts, as `User=25,Post=200`
        #[arg(long, value_name = "PAIRS")]
        counts: Option<String>,

        #[arg(long)]
        cors: bool,

        /// Repair malformed descriptions rather than refusing the file, and
        /// report every repair
        #[arg(long)]
        lenient: bool,

        #[arg(short, long)]
        verbose: bool,
    },

    /// Print the entity graph a spec compiles to, and what it cost
    Explain {
        /// GraphQL schema file (.graphql / .gql)
        spec: PathBuf,

        #[arg(long, default_value = "0")]
        seed: u64,

        #[arg(long, default_value = "12")]
        count: usize,

        #[arg(long, value_name = "PAIRS")]
        counts: Option<String>,

        /// Repair malformed descriptions rather than refusing the file, and
        /// report every repair
        #[arg(long)]
        lenient: bool,
    },
}

pub async fn execute(command: SpecCommand) -> anyhow::Result<()> {
    match command.action {
        SpecAction::Serve {
            spec,
            port,
            host,
            endpoint,
            seed,
            count,
            counts,
            cors,
            lenient,
            verbose,
        } => {
            let compiled = compile(&spec, seed, count, counts.as_deref(), lenient)?;
            report(&compiled);

            crate::say!();
            crate::say!(
                "{}",
                ui::success(&format!(
                    "GraphQL backend on http://{host}:{port}{endpoint}"
                ))
            );

            let mock = ferrimock::spec::emit::mount_graphql(
                Arc::clone(&compiled.backend),
                &endpoint,
            );

            super::serve::serve_mock_server(super::serve::MockServerConfig {
                port,
                host,
                mocks_dir: None,
                mock_file: None,
                watch: false,
                cors,
                enable_render_endpoint: false,
                log_matches: verbose,
                verbose,
                open_browser: false,
                explain_unmatched: true,
                extra_mocks: vec![mock],
            })
            .await
        }

        SpecAction::Explain {
            spec,
            seed,
            count,
            counts,
            lenient,
        } => {
            let compiled = compile(&spec, seed, count, counts.as_deref(), lenient)?;
            report(&compiled);
            explain(&compiled);
            Ok(())
        }
    }
}

struct Compiled {
    graph: Arc<EntityGraph>,
    store: Arc<EntityStore>,
    backend: Arc<GraphQLBackend>,
}

fn compile(
    path: &Path,
    seed: u64,
    default_count: usize,
    counts: Option<&str>,
    lenient: bool,
) -> anyhow::Result<Compiled> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Could not read {}", path.display()))?;

    let parsed = if lenient {
        let (parsed, repaired) = parse_sdl_lenient(&source)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("Could not parse {}", path.display()))?;
        if !repaired.is_empty() {
            crate::say!(
                "{}",
                ui::warning(&format!(
                    "repaired {} malformed description(s) to read this file; the file itself is \
                     still invalid GraphQL and should be regenerated",
                    repaired.len()
                ))
            );
            for defect in repaired.iter().take(3) {
                crate::say!("{}", ui::dim(&format!("  {defect}")));
            }
        }
        parsed
    } else {
        parse_sdl(&source)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("Could not parse {}", path.display()))?
    };

    let graph = Arc::new(to_entity_graph(&parsed));

    let mut config = StoreConfig::seeded(seed);
    config.default_count = default_count;
    for (entity, count) in parse_counts(counts)? {
        config = config.with_count(entity, count);
    }

    let store = Arc::new(EntityStore::new(Arc::clone(&graph), config));
    let backend = Arc::new(
        GraphQLBackend::build(&parsed, Arc::clone(&store)).map_err(|e| anyhow::anyhow!("{e}"))?,
    );

    Ok(Compiled {
        graph,
        store,
        backend,
    })
}

fn parse_counts(counts: Option<&str>) -> anyhow::Result<Vec<(String, usize)>> {
    let Some(counts) = counts else {
        return Ok(Vec::new());
    };
    counts
        .split(',')
        .filter(|pair| !pair.trim().is_empty())
        .map(|pair| {
            let (entity, count) = pair
                .split_once('=')
                .with_context(|| format!("`{pair}` should be written `Entity=count`"))?;
            let parsed = count
                .trim()
                .parse::<usize>()
                .with_context(|| format!("`{count}` is not a count"))?;
            Ok((entity.trim().to_string(), parsed))
        })
        .collect()
}

/// The headline every run leads with: how much of the API is store-backed.
fn report(compiled: &Compiled) {
    let coverage = compiled.backend.coverage();
    let total = coverage.classified().len() + coverage.unclassified().len();

    crate::say!("{}", ui::header("Spec"));
    crate::say!();
    crate::say!(
        "{}",
        ui::info(&format!(
            "{} entities, {} root fields, {} store-backed ({:.0}%)",
            compiled.graph.len(),
            total,
            coverage.classified().len(),
            coverage.ratio() * 100.0
        ))
    );

    for note in coverage.unsupported() {
        crate::say!("{}", ui::warning(&format!("not served: {note}")));
    }

    let dropped = coverage.dropped_interfaces();
    if !dropped.is_empty() {
        crate::say!(
            "{}",
            ui::warning(&format!(
                "{} interface implementation(s) dropped: the schema builder refuses the \
                 covariant field types GraphQL allows (e.g. {})",
                dropped.len(),
                dropped
                    .iter()
                    .take(2)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        );
    }

    let unclassified = coverage.unclassified();
    if !unclassified.is_empty() {
        let shown = unclassified
            .iter()
            .take(8)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let more = unclassified.len().saturating_sub(8);
        let tail = if more > 0 {
            format!(", and {more} more")
        } else {
            String::new()
        };
        crate::say!(
            "{}",
            ui::warning(&format!(
                "{} field(s) answered from their declared shape alone: {shown}{tail}",
                unclassified.len()
            ))
        );
    }
}

fn explain(compiled: &Compiled) {
    crate::say!();
    crate::say!("{}", ui::header("Entities"));

    for entity in compiled.graph.entities() {
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
                compiled.store.count(entity.name.as_str()),
                entity.provenance.rule
            ))
        );

        for field in &entity.fields {
            let Some(relation) = field.relation() else {
                continue;
            };
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

        let scalars = entity
            .value_fields()
            .filter(|f| matches!(f.value, ValueSpec::Scalar(_) | ValueSpec::Enum(_)))
            .count();
        if scalars > 0 {
            crate::say!("  {} {scalars} value field(s)", ui::dim("·"));
        }
    }

    // Only an unsatisfiable cut is worth a reader's attention: a to-many edge
    // can always be empty, so breaking one costs nothing.
    let order = compiled.graph.seed_order();
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn counts_parse_into_pairs() {
        let parsed = parse_counts(Some("User=25, Post=200")).unwrap();
        assert_eq!(
            parsed,
            vec![("User".to_string(), 25), ("Post".to_string(), 200)]
        );
    }

    #[test]
    fn no_counts_is_not_an_error() {
        assert!(parse_counts(None).unwrap().is_empty());
        assert!(parse_counts(Some("")).unwrap().is_empty());
    }

    #[test]
    fn a_malformed_count_says_so() {
        assert!(parse_counts(Some("User")).is_err());
        assert!(parse_counts(Some("User=lots")).is_err());
    }
}
