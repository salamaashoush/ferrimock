//! REST binding: an OpenAPI document over the entity world.
//!
//! Unlike GraphQL, which mounts as one endpoint because the client chooses the
//! operation name, a document *designs* endpoints — so this produces one bound
//! operation per method and path, and the emitter turns each into its own
//! mock. That is what lets coverage name the endpoints, `verify()` assert on
//! one of them, and an override be an ordinary higher-priority mock.

pub mod answer;
pub mod classify;

use lean_string::LeanString;
use rustc_hash::FxHashSet;
use std::sync::Arc;

use answer::{BoundOperation, Coverage, EnvelopeSlot, Pagination, ParentLink, filterable_fields};
use classify::classify;

use crate::core::World;
use crate::core::world::model::EntityGraph;
use crate::profile::{ConsolidationProfile, DefaultProfile};
use crate::spec::bind::plan::RootPlan;
use crate::spec::infer::openapi::document::{
    Operation, OperationTable, ParamIn, SchemaKind, SchemaRef,
};
use crate::spec::infer::openapi::entities::subresource_parent;
use crate::spec::infer::openapi::schema::{Lens, value_spec_of};

/// Query parameter names that page a list, when the profile names none.
const LIMIT_NAMES: [&str; 6] = ["limit", "per_page", "page_size", "first", "count", "take"];
const OFFSET_NAMES: [&str; 4] = ["offset", "skip", "start", "from"];
const PAGE_NAMES: [&str; 2] = ["page", "page_number"];
const SORT_NAMES: [&str; 4] = ["sort", "sort_by", "order_by", "orderBy"];

/// Envelope field names holding the size of the whole collection.
const TOTAL_NAMES: [&str; 5] = ["total", "total_count", "totalCount", "count", "size"];

/// Every operation of one document, bound to one world.
pub struct RestBackend {
    pub operations: Vec<Arc<BoundOperation>>,
    coverage: Arc<Coverage>,
}

impl RestBackend {
    /// Bind a document to a world.
    pub fn build(table: &Arc<OperationTable>, world: &Arc<World>) -> Self {
        Self::build_with(table, world, &DefaultProfile)
    }

    /// [`Self::build`] with a profile naming this API's pagination dialect.
    pub fn build_with(
        table: &Arc<OperationTable>,
        world: &Arc<World>,
        profile: &dyn ConsolidationProfile,
    ) -> Self {
        let graph = world.graph();
        let entity_names: FxHashSet<LeanString> =
            graph.entities().map(|entity| entity.name.clone()).collect();
        let pagination = pagination_names(profile);

        let mut coverage = Coverage::default();
        let mut plans = Vec::with_capacity(table.operations.len());
        for operation in &table.operations {
            let plan = classify(table, operation, &graph);
            coverage.record(operation.id.as_str(), &plan);
            plans.push(plan);
        }
        let coverage = Arc::new(coverage);

        let lens = Lens {
            book: &table.schemas,
            entities: &entity_names,
            profile,
        };

        let operations = table
            .operations
            .iter()
            .zip(plans)
            .map(|(operation, plan)| {
                Arc::new(bind_one(
                    operation,
                    plan,
                    table,
                    &graph,
                    &lens,
                    world,
                    &coverage,
                    &pagination,
                ))
            })
            .collect();

        Self {
            operations,
            coverage,
        }
    }

    /// How much of the document is answered from the store.
    #[must_use]
    pub fn coverage(&self) -> &Arc<Coverage> {
        &self.coverage
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_one(
    operation: &Operation,
    plan: RootPlan,
    table: &OperationTable,
    graph: &Arc<EntityGraph>,
    lens: &Lens<'_>,
    world: &Arc<World>,
    coverage: &Arc<Coverage>,
    pagination: &Pagination,
) -> BoundOperation {
    let response = operation.success();
    let status = response
        .and_then(|response| http::StatusCode::from_u16(response.status.status_code()).ok())
        .unwrap_or(http::StatusCode::OK);
    let content_type = response
        .and_then(|response| response.content_type.clone())
        .unwrap_or_else(|| LeanString::from("application/json"));

    // Only the bottom rung needs the declared shape, and building one for every
    // operation of a 500-operation document is wasted work.
    let declared = matches!(plan, RootPlan::Unclassified)
        .then(|| {
            response
                .and_then(|response| response.schema.as_ref())
                .map(|schema| {
                    Arc::new(value_spec_of(
                        lens,
                        schema,
                        operation.id.as_str(),
                        operation.id.as_str(),
                    ))
                })
        })
        .flatten();

    let envelope = match (&plan, response.and_then(|r| r.schema.as_ref())) {
        (RootPlan::List { payload_field, .. }, Some(schema)) => {
            envelope_of(table, schema, payload_field.as_ref(), lens)
        }
        _ => Vec::new(),
    };

    let parent = match &plan {
        RootPlan::List { entity, .. } => parent_link(operation, graph, entity),
        _ => None,
    };

    let filterable = match (&plan, plan.entity()) {
        (RootPlan::List { .. }, Some(entity)) => {
            let declared: FxHashSet<&str> = operation
                .parameters
                .iter()
                .filter(|parameter| parameter.location == ParamIn::Query)
                .map(|parameter| parameter.name.as_str())
                .collect();
            graph.get(entity.as_str()).map_or_else(Vec::new, |entity| {
                let mut fields = filterable_fields(entity);
                // A document that lists its query parameters has said which
                // ones it takes; one that lists none leaves every field open,
                // which is the useful default for a hand-written document.
                if !declared.is_empty() {
                    fields.retain(|(name, _)| declared.contains(name.as_str()));
                }
                fields
            })
        }
        _ => Vec::new(),
    };

    BoundOperation {
        id: operation.id.clone(),
        method: operation.method.clone(),
        path: operation.path.clone(),
        summary: operation.summary.clone(),
        plan,
        status,
        content_type,
        world: Arc::clone(world),
        coverage: Arc::clone(coverage),
        declared,
        envelope,
        parent,
        pagination: pagination.clone(),
        filterable,
    }
}

/// What each property of a list's response object holds.
fn envelope_of(
    table: &OperationTable,
    schema: &SchemaRef,
    payload_field: Option<&LeanString>,
    lens: &Lens<'_>,
) -> Vec<(LeanString, EnvelopeSlot)> {
    let Some(payload_field) = payload_field else {
        return Vec::new();
    };
    let Some(node) = table.schemas.effective(schema) else {
        return Vec::new();
    };
    if node.effective_kind() != SchemaKind::Object {
        return Vec::new();
    }

    node.properties
        .iter()
        .map(|property| {
            let slot = if property.name == *payload_field {
                EnvelopeSlot::Records
            } else if is_named(&property.name, &TOTAL_NAMES) {
                EnvelopeSlot::Total
            } else if is_named(&property.name, &LIMIT_NAMES) {
                EnvelopeSlot::Limit
            } else if is_named(&property.name, &OFFSET_NAMES) {
                EnvelopeSlot::Offset
            } else {
                EnvelopeSlot::Declared(Arc::new(value_spec_of(
                    lens,
                    &property.schema,
                    property.name.as_str(),
                    "envelope",
                )))
            };
            (property.name.clone(), slot)
        })
        .collect()
}

fn is_named(name: &LeanString, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

/// The parent a nested list hangs off, when the graph says one does.
///
/// `/folders/{folder_id}/items` reads the children through the relation
/// inference already put on `Folder`, rather than filtering the world by a
/// foreign key that may not exist.
fn parent_link(
    operation: &Operation,
    graph: &EntityGraph,
    child: &LeanString,
) -> Option<ParentLink> {
    let (entity, param, field) = subresource_parent(graph, operation, Some(child))?;
    Some(ParentLink {
        entity: entity.name.clone(),
        param,
        field,
    })
}

fn pagination_names(profile: &dyn ConsolidationProfile) -> Pagination {
    let dialect = profile.pagination_dialect();
    let names = |declared: Option<&Vec<String>>, built_in: &[&str]| -> Vec<LeanString> {
        // The profile's names go first: an API that calls its cursor
        // `continuation` is understood without the engine having to guess, and
        // the built-ins stay as a fallback rather than a competitor.
        let mut names: Vec<LeanString> = declared
            .into_iter()
            .flatten()
            .map(|name| LeanString::from(name.as_str()))
            .collect();
        names.extend(built_in.iter().copied().map(LeanString::from));
        names
    };

    Pagination {
        limit: names(dialect.map(|d| &d.limit), &LIMIT_NAMES),
        offset: names(dialect.map(|d| &d.offset), &OFFSET_NAMES),
        page: names(None, &PAGE_NAMES),
        sort: names(None, &SORT_NAMES),
    }
}
