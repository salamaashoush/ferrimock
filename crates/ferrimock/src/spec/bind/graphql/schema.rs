//! Building an executable schema from a parsed one.
//!
//! Every type the schema declares is registered with `async_graphql::dynamic`,
//! which gives selection sets, fragments, variables, directives, error paths
//! and introspection for free — so a client, a codegen tool or GraphiQL points
//! at the mock exactly as it points at the real service.
//!
//! Resolvers come in two kinds, because root fields and entity fields are not
//! the same problem. An entity field is generic: take the parent record, read
//! the field, and follow it into the store if it is a link. A root field is
//! classified once at build time (see [`super::classify`]) and resolves
//! according to its rung.

use async_graphql::dynamic::{
    Enum, EnumItem, Field, FieldFuture, FieldValue, InputObject, InputValue, Interface,
    InterfaceField, Object, ResolverContext, Scalar, Schema, TypeRef as DynTypeRef, Union,
};
use async_graphql::{Request, Response, Value as GqlValue};
use lean_string::LeanString;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::classify::{self, RootKind, RootPlan};
use super::value::{to_gql, to_json};
use crate::core::World;
use crate::core::world::algebra::{Cursor, Mutation, Page, Predicate, Selection, SortKey};
use crate::core::world::model::{Carrier, EntityGraph, EntityKey, ValueSpec};
use crate::core::world::store::values::{ValueSeed, generate};
use crate::core::world::store::{EntityStore, PageResult, Record, Written};
use crate::graphql::introspection::{
    FieldDefinition, ParsedSchema, TypeDefinition, TypeKind, TypeRef,
};
use crate::spec::infer::graphql::entities::{SchemaFacts, value_spec_of};

/// How many instances a list returns when the query does not say.
const DEFAULT_PAGE_SIZE: usize = 10;

/// What a resolver hands to the resolvers below it.
#[derive(Debug, Clone)]
enum Parent {
    /// A stored instance.
    Entity(Record),
    /// An inlined value object.
    Value(JsonValue),
    /// A page of instances, for the connection machinery.
    Page(Arc<PageResult>),
    /// One element of a connection.
    Edge { record: Record, cursor: String },
    /// A mutation payload wrapping what the write produced.
    Payload(Box<Parent>),
}

/// Which root fields could not be classified, and how often the fallback ran.
///
/// A mock that invents data for half a schema looks identical to one that does
/// not, unless it says so. This is what it says it with.
#[derive(Debug, Default)]
pub struct Coverage {
    unclassified_fields: Vec<String>,
    classified_fields: Vec<String>,
    unsupported: Vec<String>,
    dropped_interfaces: Vec<String>,
    fallback_hits: AtomicU64,
}

impl Coverage {
    /// Root fields answered from the store.
    #[must_use]
    pub fn classified(&self) -> &[String] {
        &self.classified_fields
    }

    /// Root fields answered from their declared shape alone.
    #[must_use]
    pub fn unclassified(&self) -> &[String] {
        &self.unclassified_fields
    }

    /// Parts of the schema this backend does not serve at all.
    #[must_use]
    pub fn unsupported(&self) -> &[String] {
        &self.unsupported
    }

    /// `implements` declarations dropped because the schema builder refuses
    /// the covariance the GraphQL spec allows. One systemic limitation, not
    /// one problem per type — reported as a count with examples.
    #[must_use]
    pub fn dropped_interfaces(&self) -> &[String] {
        &self.dropped_interfaces
    }

    /// How many requests have been answered by the fallback so far.
    #[must_use]
    pub fn fallback_hits(&self) -> u64 {
        self.fallback_hits.load(Ordering::Relaxed)
    }

    /// The share of root fields backed by the store, 0.0 to 1.0.
    #[must_use]
    pub fn ratio(&self) -> f64 {
        let total = self.classified_fields.len() + self.unclassified_fields.len();
        if total == 0 {
            return 1.0;
        }
        f64::from(u32::try_from(self.classified_fields.len()).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(total).unwrap_or(u32::MAX))
    }
}

/// An executable GraphQL schema backed by the engine's entity world.
///
/// Holds the world rather than a store: adding a schema rebuilds the store,
/// and a backend that had captured the old `Arc` would go on serving a world
/// nobody can write to any more.
pub struct GraphQLBackend {
    schema: Schema,
    coverage: Arc<Coverage>,
    world: Arc<World>,
}

impl std::fmt::Debug for GraphQLBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphQLBackend")
            .field("coverage", &self.coverage.ratio())
            .finish_non_exhaustive()
    }
}

impl GraphQLBackend {
    /// Register every declared type and wire its resolvers.
    pub fn build(parsed: &ParsedSchema, world: Arc<World>) -> crate::Result<Self> {
        let store = world.store();
        let mut coverage = Coverage::default();
        let query_root = parsed
            .query_type
            .clone()
            .ok_or_else(|| crate::mp_err!("Schema has no query root"))?;

        // Subscriptions are not served: async-graphql needs a subscription
        // root built from `dynamic::Subscription`, and the store has no
        // change-notification mechanism to drive one. Dropping the root is
        // what lets a schema that declares subscriptions still serve its
        // queries and mutations, instead of refusing the whole file.
        if let Some(root) = parsed.subscription_type.as_deref() {
            coverage
                .unsupported
                .push(format!("{root} (subscriptions are not served)"));
        }

        let mut builder = Schema::build(query_root.as_str(), parsed.mutation_type.as_deref(), None);

        // Sorted so a schema always registers in the same order; a build that
        // depends on hash iteration order is a build that fails intermittently.
        let mut definitions: Vec<&TypeDefinition> = parsed.types.values().collect();
        definitions.sort_by(|a, b| a.name.cmp(&b.name));

        let graph = store.graph();
        let facts = SchemaFacts::of(parsed);
        for definition in definitions {
            if definition.name.starts_with("__") {
                continue;
            }
            if Some(definition.name.as_str()) == parsed.subscription_type.as_deref() {
                continue;
            }
            let is_root = Some(definition.name.as_str()) == parsed.query_type.as_deref()
                || Some(definition.name.as_str()) == parsed.mutation_type.as_deref();

            match definition.kind {
                TypeKind::Object if is_root => {
                    let kind = if Some(definition.name.as_str()) == parsed.query_type.as_deref() {
                        RootKind::Query
                    } else {
                        RootKind::Mutation
                    };
                    builder = builder.register(root_object(
                        definition,
                        kind,
                        parsed,
                        graph,
                        &facts,
                        &mut coverage,
                    ));
                }
                TypeKind::Object => {
                    builder =
                        builder.register(object(definition, parsed, graph, &facts, &mut coverage));
                }
                TypeKind::Interface => builder = builder.register(interface(definition)),
                TypeKind::Union => {
                    let mut union = Union::new(definition.name.as_str());
                    for member in &definition.possible_types {
                        union = union.possible_type(member.name());
                    }
                    builder = builder.register(union);
                }
                TypeKind::Enum => {
                    let mut enumeration = Enum::new(definition.name.as_str());
                    for value in &definition.enum_values {
                        enumeration = enumeration.item(EnumItem::new(value.name.as_str()));
                    }
                    builder = builder.register(enumeration);
                }
                TypeKind::InputObject => {
                    let mut input = InputObject::new(definition.name.as_str());
                    for field in &definition.input_fields {
                        input = input.field(InputValue::new(
                            field.name.as_str(),
                            dyn_type_ref(&field.value_type),
                        ));
                    }
                    builder = builder.register(input);
                }
                TypeKind::Scalar => {
                    // The five built-ins are registered by async-graphql; a
                    // second registration is a build error, not a no-op.
                    if !matches!(
                        definition.name.as_str(),
                        "String" | "Int" | "Float" | "Boolean" | "ID"
                    ) {
                        builder = builder.register(Scalar::new(definition.name.as_str()));
                    }
                }
                TypeKind::List | TypeKind::NonNull => {}
            }
        }

        let coverage = Arc::new(coverage);
        let schema = builder
            .data(Arc::clone(&world))
            .data(Arc::clone(&coverage))
            .finish()
            .map_err(|e| crate::mp_err!("Could not build an executable schema: {e}"))?;

        Ok(Self {
            schema,
            coverage,
            world,
        })
    }

    pub async fn execute(&self, request: Request) -> Response {
        self.schema.execute(request).await
    }

    /// The schema as SDL, which is what a client's codegen reads.
    #[must_use]
    pub fn sdl(&self) -> String {
        self.schema.sdl()
    }

    #[must_use]
    pub fn coverage(&self) -> &Arc<Coverage> {
        &self.coverage
    }

    /// The world this backend serves. Shared with every other lane.
    #[must_use]
    pub fn world(&self) -> &Arc<World> {
        &self.world
    }
}

/// Whether a named type is an interface or a union.
fn is_abstract_type(parsed: &ParsedSchema, name: &str) -> bool {
    parsed
        .types
        .get(name)
        .is_some_and(|def| matches!(def.kind, TypeKind::Interface | TypeKind::Union))
}

/// One stored instance as a resolver value.
///
/// A field declared as an interface or union must say which concrete type it
/// produced — async-graphql cannot infer it from an opaque parent value. A
/// field declared as a concrete type must *not*: the type wrapper hides the
/// value from the resolvers below, which then have nothing to read.
fn entity_value(record: Record, abstract_field: bool) -> FieldValue<'static> {
    if abstract_field {
        let typename = record.entity.to_string();
        return FieldValue::owned_any(Parent::Entity(record)).with_type(typename);
    }
    FieldValue::owned_any(Parent::Entity(record))
}

fn dyn_type_ref(type_ref: &TypeRef) -> DynTypeRef {
    match type_ref {
        TypeRef::Named(name) => DynTypeRef::Named(name.clone().into()),
        TypeRef::NonNull(inner) => DynTypeRef::NonNull(Box::new(dyn_type_ref(inner))),
        TypeRef::List(inner) => DynTypeRef::List(Box::new(dyn_type_ref(inner))),
    }
}

/// Whether `async_graphql::dynamic` will accept this `implements`.
///
/// GraphQL allows an implementing field to return a *subtype* of the
/// interface's field type — `[ActionEdge!]!` satisfies `[EdgeInterface]!` when
/// `ActionEdge implements EdgeInterface`. The dynamic builder's check compares
/// named types by equality alone (`dynamic/type_ref.rs`), so it rejects that.
/// This mirrors its rule exactly, so only the declarations it would refuse are
/// dropped.
fn covariance_is_registrable(
    definition: &TypeDefinition,
    interface_name: &str,
    parsed: &ParsedSchema,
) -> bool {
    let Some(interface) = parsed.types.get(interface_name) else {
        return false;
    };

    interface.fields.iter().all(|declared| {
        definition
            .fields
            .iter()
            .find(|f| f.name == declared.name)
            .is_some_and(|implemented| {
                builder_accepts_override(&implemented.field_type, &declared.field_type)
            })
    })
}

/// An exact mirror of `dynamic::TypeRef::is_subtype` as the builder calls it
/// (`impl_field.ty().is_subtype(&interface_field.ty)`).
///
/// Two ways it departs from the GraphQL spec, both of which real schemas trip:
/// named types are compared by equality, so an implementor returning a type
/// that *implements* the interface's field type is refused; and the
/// nullability comparison is inverted, so `cursor: String!` implementing
/// `cursor: String` — the legal, common case — is refused while the unsound
/// reverse is allowed. Reproducing the rule rather than the spec is the point:
/// only declarations the builder would reject get dropped.
fn builder_accepts_override(implemented: &TypeRef, declared: &TypeRef) -> bool {
    match (implemented, declared) {
        (TypeRef::NonNull(implemented), TypeRef::NonNull(declared))
        | (TypeRef::List(implemented), TypeRef::List(declared)) => {
            builder_accepts_override(implemented, declared)
        }
        (_, TypeRef::NonNull(declared)) => builder_accepts_override(implemented, declared),
        (TypeRef::Named(implemented), TypeRef::Named(declared)) => implemented == declared,
        _ => false,
    }
}

fn interface(definition: &TypeDefinition) -> Interface {
    let mut iface = Interface::new(definition.name.as_str());
    for field in &definition.fields {
        iface = iface.field(InterfaceField::new(
            field.name.as_str(),
            dyn_type_ref(&field.field_type),
        ));
    }
    iface
}

/// A non-root object: entity fields read the parent record, value-object
/// fields read the parent JSON, and connection machinery reads the page.
fn object(
    definition: &TypeDefinition,
    parsed: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
    coverage: &mut Coverage,
) -> Object {
    let mut object = Object::new(definition.name.as_str());
    for interface_ref in &definition.interfaces {
        if covariance_is_registrable(definition, interface_ref.name(), parsed) {
            object = object.implement(interface_ref.name());
        } else {
            // The schema is right and the builder is strict; keeping the
            // object's declared field types matters more than keeping an
            // `implements` edge, because those types are what a client
            // generates code from.
            coverage.dropped_interfaces.push(format!(
                "{} implements {}",
                definition.name,
                interface_ref.name()
            ));
        }
    }

    let owner = LeanString::from(definition.name.as_str());
    for field_def in &definition.fields {
        object = object.field(entity_field(&owner, field_def, parsed, graph, facts));
    }
    object
}

fn entity_field(
    owner: &LeanString,
    field_def: &FieldDefinition,
    parsed: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
) -> Field {
    let owner = owner.clone();
    let field_name = LeanString::from(field_def.name.as_str());
    let type_ref = dyn_type_ref(&field_def.field_type);
    let named_type = LeanString::from(field_def.field_type.name());
    let returns_entity = graph.contains(named_type.as_str());
    let is_abstract = is_abstract_type(parsed, named_type.as_str());
    let connection_of = connection_node(parsed, graph, named_type.as_str());

    let is_list = field_def.field_type.is_list();
    // A non-null field that nothing sourced would abort the whole selection,
    // so it falls back to a value of its declared shape. Nullable fields keep
    // returning null: inventing a value where null is legal would be worse
    // than the hole it fills.
    let non_null = field_def.field_type.is_non_null();
    let declared: Option<Arc<ValueSpec>> = non_null.then(|| {
        Arc::new(value_spec_of(
            parsed,
            facts,
            &field_def.field_type,
            &field_def.name,
        ))
    });

    let mut field = Field::new(field_def.name.as_str(), type_ref, move |ctx| {
        let owner = owner.clone();
        let field_name = field_name.clone();
        let named_type = named_type.clone();
        let connection_of = connection_of.clone();
        let declared = declared.clone();
        FieldFuture::new(async move {
            let resolved = resolve_entity_field(
                &ctx,
                &owner,
                &field_name,
                &named_type,
                returns_entity,
                is_list,
                is_abstract,
                connection_of.as_deref(),
            )?;
            if resolved.is_some() {
                return Ok(resolved);
            }
            let Some(declared) = declared else {
                return Ok(None);
            };
            let seed = ValueSeed::new(seed_of(&ctx), "__declared", args_ordinal(&ctx));
            Ok(Some(json_field_value(&generate(
                &declared,
                &field_name,
                seed,
            ))))
        })
    });

    for arg in &field_def.args {
        field = field.argument(InputValue::new(
            arg.name.as_str(),
            dyn_type_ref(&arg.value_type),
        ));
    }
    field
}

/// The entity a connection type wraps, when the named type is one.
fn connection_node(
    parsed: &ParsedSchema,
    graph: &EntityGraph,
    type_name: &str,
) -> Option<LeanString> {
    let definition = parsed.types.get(type_name)?;
    let edges = definition.fields.iter().find(|f| f.name == "edges")?;
    definition.fields.iter().find(|f| f.name == "pageInfo")?;
    let edge = parsed.types.get(edges.field_type.name())?;
    let node = edge.fields.iter().find(|f| f.name == "node")?;
    graph
        .contains(node.field_type.name())
        .then(|| LeanString::from(node.field_type.name()))
}

#[allow(clippy::too_many_arguments)]
fn resolve_entity_field(
    ctx: &ResolverContext<'_>,
    owner: &str,
    field_name: &str,
    named_type: &str,
    returns_entity: bool,
    is_list: bool,
    abstract_field: bool,
    connection_of: Option<&str>,
) -> async_graphql::Result<Option<FieldValue<'static>>> {
    let parent = ctx
        .parent_value
        .try_downcast_ref::<Parent>()
        .map_err(|_| async_graphql::Error::new("Resolver reached a value it cannot read"))?;

    match parent {
        // A payload object holds one thing: what the operation produced —
        // a created record, or the page a list wrapper carries. Its other
        // fields (`errors`, `clientMutationId`) have no source, and a non-null
        // list still has to be a list rather than a null.
        Parent::Payload(inner) => {
            if returns_entity || abstract_field {
                return Ok(Some(match &**inner {
                    Parent::Entity(record) => entity_value(record.clone(), abstract_field),
                    Parent::Page(page) => FieldValue::list(
                        page.records
                            .iter()
                            .cloned()
                            .map(|record| entity_value(record, abstract_field)),
                    ),
                    other => FieldValue::owned_any(other.clone()),
                }));
            }
            if connection_of.is_some()
                && let Parent::Page(page) = &**inner
            {
                return Ok(Some(FieldValue::owned_any(Parent::Page(Arc::clone(page)))));
            }
            Ok(is_list.then(|| FieldValue::list(std::iter::empty::<FieldValue<'static>>())))
        }
        Parent::Entity(record) => {
            let store = store_of(ctx)?;
            resolve_on_record(
                ctx,
                &store,
                record,
                owner,
                field_name,
                returns_entity,
                abstract_field,
                connection_of,
            )
        }
        Parent::Value(value) => Ok(field_of_json(value, field_name)),
        Parent::Page(page) => Ok(Some(resolve_on_page(
            page,
            field_name,
            named_type,
            abstract_field,
        ))),
        Parent::Edge { record, cursor } => match field_name {
            "node" => Ok(Some(entity_value(record.clone(), abstract_field))),
            "cursor" => Ok(Some(FieldValue::value(GqlValue::String(cursor.clone())))),
            _ => Ok(None),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_on_record(
    ctx: &ResolverContext<'_>,
    store: &Arc<EntityStore>,
    record: &Record,
    owner: &str,
    field_name: &str,
    returns_entity: bool,
    abstract_field: bool,
    connection_of: Option<&str>,
) -> async_graphql::Result<Option<FieldValue<'static>>> {
    let relation = store
        .graph()
        .get(owner)
        .and_then(|entity| entity.field(field_name))
        .and_then(|field| field.relation().map(|r| (field, r)));

    let returns_link = returns_entity || abstract_field;
    let Some((field_def, relation)) = relation else {
        // Not a link: the value is already on the record, or it is a field the
        // store does not carry (a connection wrapper on a value object).
        if let Some(node) = connection_of {
            let _ = node;
        }
        return Ok(field_of_json(
            &JsonValue::Object(record.fields.clone()),
            field_name,
        ));
    };

    let is_connection = matches!(relation.carrier, Carrier::Connection(_));
    let selection = selection_from_args(ctx, store, relation.target.as_str());

    if returns_link && !field_def.value.is_list() && !is_connection {
        let target = store.relation_target(owner, &record.key, field_name, relation);
        return Ok(target.map(|found| entity_value(found, abstract_field)));
    }

    let page = store
        .related(owner, &record.key, field_name, &selection)
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;

    if is_connection {
        return Ok(Some(FieldValue::owned_any(Parent::Page(Arc::new(page)))));
    }
    Ok(Some(FieldValue::list(
        page.records
            .into_iter()
            .map(|record| entity_value(record, abstract_field)),
    )))
}

fn resolve_on_page(
    page: &Arc<PageResult>,
    field_name: &str,
    named_type: &str,
    abstract_field: bool,
) -> FieldValue<'static> {
    let value = match field_name {
        "edges" => {
            return FieldValue::list(page.records.iter().map(|record| {
                FieldValue::owned_any(Parent::Edge {
                    record: record.clone(),
                    cursor: record.key.to_string(),
                })
            }));
        }
        "nodes" => {
            return FieldValue::list(
                page.records
                    .iter()
                    .cloned()
                    .map(|record| entity_value(record, abstract_field)),
            );
        }
        "pageInfo" => {
            let _ = named_type;
            return FieldValue::owned_any(Parent::Page(Arc::clone(page)));
        }
        "totalCount" => GqlValue::Number(page.total.into()),
        "hasNextPage" => GqlValue::Boolean(page.has_next),
        "hasPreviousPage" => GqlValue::Boolean(page.has_previous),
        "startCursor" => page
            .start_cursor
            .as_ref()
            .map_or(GqlValue::Null, |c| GqlValue::String(c.as_str().to_string())),
        "endCursor" => page
            .end_cursor
            .as_ref()
            .map_or(GqlValue::Null, |c| GqlValue::String(c.as_str().to_string())),
        _ => GqlValue::Null,
    };
    FieldValue::value(value)
}

/// Read a field off an inlined value, keeping objects walkable.
fn field_of_json(value: &JsonValue, field_name: &str) -> Option<FieldValue<'static>> {
    let field = value.get(field_name)?;
    Some(match field {
        JsonValue::Object(_) => FieldValue::owned_any(Parent::Value(field.clone())),
        JsonValue::Array(items) => FieldValue::list(items.iter().map(|item| match item {
            JsonValue::Object(_) => FieldValue::owned_any(Parent::Value(item.clone())),
            other => FieldValue::value(to_gql(other)),
        })),
        JsonValue::Null => return None,
        other => FieldValue::value(to_gql(other)),
    })
}

fn root_object(
    definition: &TypeDefinition,
    kind: RootKind,
    parsed: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
    coverage: &mut Coverage,
) -> Object {
    let mut object = Object::new(definition.name.as_str());

    for field_def in &definition.fields {
        let plan = classify::classify(field_def, kind, parsed, graph, facts);
        let label = format!("{}.{}", definition.name, field_def.name);
        if plan.is_classified() {
            coverage.classified_fields.push(label);
        } else {
            coverage.unclassified_fields.push(label);
        }
        object = object.field(root_field(field_def, plan, parsed, graph, facts));
    }
    object
}

fn root_field(
    field_def: &FieldDefinition,
    plan: RootPlan,
    parsed: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
) -> Field {
    let type_ref = dyn_type_ref(&field_def.field_type);
    let named_type = LeanString::from(field_def.field_type.name());
    let return_shape = ReturnShape::of(field_def, parsed, graph, facts);

    let mut field = Field::new(field_def.name.as_str(), type_ref, move |ctx| {
        let plan = plan.clone();
        let named_type = named_type.clone();
        let return_shape = return_shape.clone();
        FieldFuture::new(async move { resolve_root(&ctx, &plan, &named_type, &return_shape) })
    });

    for arg in &field_def.args {
        field = field.argument(InputValue::new(
            arg.name.as_str(),
            dyn_type_ref(&arg.value_type),
        ));
    }
    field
}

/// Enough about a root field's return type to answer it without the store.
#[derive(Debug, Clone)]
struct ReturnShape {
    is_list: bool,
    is_connection: bool,
    is_abstract: bool,
    /// The payload object's own fields, when the entity is wrapped.
    payload: Option<Arc<TypeDefinition>>,
    /// How to build a value of the declared return type when nothing about
    /// the field says where a real one would come from.
    declared: Arc<ValueSpec>,
}

impl ReturnShape {
    fn of(
        field_def: &FieldDefinition,
        parsed: &ParsedSchema,
        graph: &EntityGraph,
        facts: &SchemaFacts<'_>,
    ) -> Self {
        let named = field_def.field_type.name();
        let payload = parsed
            .types
            .get(named)
            .filter(|def| def.kind == TypeKind::Object && !graph.contains(named))
            .map(|def| Arc::new(def.clone()));
        Self {
            is_list: field_def.field_type.is_list(),
            is_connection: connection_node(parsed, graph, named).is_some(),
            is_abstract: is_abstract_type(parsed, named),
            payload,
            declared: Arc::new(value_spec_of(
                parsed,
                facts,
                &field_def.field_type,
                &field_def.name,
            )),
        }
    }
}

fn resolve_root(
    ctx: &ResolverContext<'_>,
    plan: &RootPlan,
    named_type: &str,
    shape: &ReturnShape,
) -> async_graphql::Result<Option<FieldValue<'static>>> {
    let store = store_of(ctx)?;

    match plan {
        RootPlan::Get {
            entity,
            members,
            key_arg,
        } => {
            let targets = concrete_or(entity, members);
            let record = argument_string(ctx, key_arg)
                .and_then(|key| store.get_any(&targets, &EntityKey::single(key)));
            Ok(record.map(|record| wrap_payload(shape, Parent::Entity(record))))
        }

        // Answered as the caller rather than as record zero. A GraphQL schema
        // has no status codes, so a request with no credential is an error
        // beside a null field, which is what a real GraphQL service answers.
        RootPlan::Viewer { entity, members } => {
            use crate::core::world::viewer::Credential;

            let targets = concrete_or(entity, members);
            let Some(bound) = viewer_of(ctx).filter(|held| targets.contains(held)) else {
                return Err(async_graphql::Error::new(format!(
                    "`{entity}` has no viewer: name one with `world.viewer` and the credential \
                     will resolve to an instance of it"
                )));
            };
            let credential = ctx
                .data::<Credential>()
                .cloned()
                .unwrap_or(Credential::Absent);
            if credential == Credential::Absent {
                return Err(async_graphql::Error::new("no credential was presented"));
            }
            let keys = store.keys(bound.as_str());
            let record = credential
                .bound_to(store.seed(), bound.as_str(), &keys)
                .and_then(|key| store.get(bound.as_str(), &key));
            Ok(record.map(|record| wrap_payload(shape, Parent::Entity(record))))
        }

        RootPlan::List {
            entity,
            members,
            connection,
            ..
        } => {
            let targets = concrete_or(entity, members);
            let selection = selection_from_args(ctx, &store, entity.as_str());
            let page = store
                .list_any(&targets, &selection)
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;

            if connection.is_some() || shape.is_connection {
                return Ok(Some(FieldValue::owned_any(Parent::Page(Arc::new(page)))));
            }
            // A list can be wrapped in a result object (`{ data: [Thing] }`);
            // the wrapper's own field is what hands the records on.
            if shape.payload.is_some() {
                return Ok(Some(FieldValue::owned_any(Parent::Payload(Box::new(
                    Parent::Page(Arc::new(page)),
                )))));
            }
            Ok(Some(FieldValue::list(
                page.records
                    .into_iter()
                    .map(|record| entity_value(record, shape.is_abstract)),
            )))
        }

        RootPlan::Create {
            entity, input_arg, ..
        } => {
            let values = input_values(ctx, input_arg.as_deref());
            let written = store
                .apply(entity.as_str(), Mutation::Insert { values })
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            Ok(written_value(shape, written))
        }

        RootPlan::Update {
            entity,
            key_arg,
            input_arg,
            ..
        } => {
            let key = argument_string(ctx, key_arg)
                .ok_or_else(|| async_graphql::Error::new(format!("`{key_arg}` is required")))?;
            let values = input_values(ctx, input_arg.as_deref());
            let written = store
                .apply(
                    entity.as_str(),
                    Mutation::Patch {
                        key: EntityKey::single(key),
                        values,
                    },
                )
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            Ok(written_value(shape, written))
        }

        RootPlan::Delete {
            entity, key_arg, ..
        } => {
            let key = argument_string(ctx, key_arg)
                .ok_or_else(|| async_graphql::Error::new(format!("`{key_arg}` is required")))?;
            let entity_key = EntityKey::single(key);
            // The deleted record is the useful answer, so read it before it
            // stops being readable.
            let removed = store.get(entity.as_str(), &entity_key);
            store
                .apply(entity.as_str(), Mutation::Remove { key: entity_key })
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            Ok(removed.map(|record| wrap_payload(shape, Parent::Entity(record))))
        }

        RootPlan::Unclassified => {
            if let Ok(coverage) = ctx.data::<Arc<Coverage>>() {
                coverage.fallback_hits.fetch_add(1, Ordering::Relaxed);
            }
            Ok(fallback_value(ctx, &store, named_type, shape))
        }
    }
}

/// A field nothing could be inferred about still has to answer. It answers
/// from its declared return type, pulling a real stored instance where the
/// type allows, so relations underneath it still resolve.
fn fallback_value(
    ctx: &ResolverContext<'_>,
    store: &Arc<EntityStore>,
    named_type: &str,
    shape: &ReturnShape,
) -> Option<FieldValue<'static>> {
    if store.graph().contains(named_type) {
        let keys = store.keys(named_type);
        if shape.is_list {
            return Some(FieldValue::list(
                keys.iter()
                    .take(DEFAULT_PAGE_SIZE)
                    .filter_map(|key| store.get(named_type, key))
                    .map(|record| entity_value(record, shape.is_abstract)),
            ));
        }
        return keys
            .first()
            .and_then(|key| store.get(named_type, key))
            .map(|record| entity_value(record, shape.is_abstract));
    }

    // Not an entity: build a value of the declared shape, seeded by the field
    // and its arguments so the same call answers the same way twice. Returning
    // nothing here is not an option — a non-null field would fail, and the
    // rung promised an answer from the declared type.
    let seed = ValueSeed::new(store.seed(), "__root", args_ordinal(ctx));
    let value = generate(&shape.declared, ctx.field().name(), seed);
    Some(json_field_value(&value))
}

/// A JSON value as a resolver value, keeping objects walkable so the fields
/// below can be read off them.
fn json_field_value(value: &JsonValue) -> FieldValue<'static> {
    match value {
        JsonValue::Object(_) => FieldValue::owned_any(Parent::Value(value.clone())),
        JsonValue::Array(items) => FieldValue::list(items.iter().map(json_field_value)),
        other => FieldValue::value(to_gql(other)),
    }
}

/// A stable ordinal for one call, so the same arguments answer the same way.
fn args_ordinal(ctx: &ResolverContext<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    ctx.field().name().hash(&mut hasher);
    let mut names: Vec<_> = ctx.args.iter().map(|(name, _)| name.to_string()).collect();
    names.sort();
    for name in names {
        name.hash(&mut hasher);
        if let Some(value) = ctx.args.get(&name) {
            to_json(value.as_value()).to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn written_value(shape: &ReturnShape, written: Written) -> Option<FieldValue<'static>> {
    match written {
        Written::Created(record) | Written::Updated(record) => {
            Some(wrap_payload(shape, Parent::Entity(record)))
        }
        Written::Removed(_) => None,
    }
}

/// A mutation payload wraps its result; the payload object's own resolvers
/// read it back out, so the parent handed down is the record either way.
fn wrap_payload(shape: &ReturnShape, parent: Parent) -> FieldValue<'static> {
    if shape.payload.is_some() {
        return FieldValue::owned_any(Parent::Payload(Box::new(parent)));
    }
    match parent {
        Parent::Entity(record) => entity_value(record, shape.is_abstract),
        other => FieldValue::owned_any(other),
    }
}

/// The store to answer this resolution from.
///
/// Read per call rather than captured at build: the world swaps its store when
/// a schema is added, and a resolver holding the old one would answer from a
/// world that no longer takes writes.
fn store_of(ctx: &ResolverContext<'_>) -> async_graphql::Result<Arc<EntityStore>> {
    Ok(ctx.data::<Arc<World>>()?.store())
}

/// The concrete entities a plan reads from: the members behind an abstract
/// type, or the type itself.
fn concrete_or(entity: &LeanString, members: &[LeanString]) -> Vec<LeanString> {
    if members.is_empty() {
        vec![entity.clone()]
    } else {
        members.to_vec()
    }
}

fn viewer_of(ctx: &ResolverContext<'_>) -> Option<LeanString> {
    ctx.data::<Arc<World>>()
        .ok()
        .and_then(|world| world.viewer())
}

fn seed_of(ctx: &ResolverContext<'_>) -> u64 {
    ctx.data::<Arc<World>>().map_or(0, |world| world.seed())
}

fn argument_string(ctx: &ResolverContext<'_>, name: &str) -> Option<String> {
    let accessor = ctx.args.get(name)?;
    match accessor.as_value() {
        GqlValue::String(s) => Some(s.clone()),
        GqlValue::Number(n) => Some(n.to_string()),
        GqlValue::Enum(name) => Some(name.to_string()),
        _ => None,
    }
}

fn input_values(ctx: &ResolverContext<'_>, input_arg: Option<&str>) -> JsonValue {
    // An input object is the values; loose arguments are the values when there
    // is no input object, which is how smaller schemas are written.
    if let Some(name) = input_arg
        && let Some(accessor) = ctx.args.get(name)
    {
        let value = to_json(accessor.as_value());
        if value.is_object() {
            return value;
        }
    }

    let mut fields = JsonMap::new();
    for (name, value) in ctx.args.iter() {
        if classify::is_pagination_arg(name) || classify::is_order_arg(name) {
            continue;
        }
        let converted = to_json(value.as_value());
        match converted {
            // A single input-object argument is the payload itself.
            JsonValue::Object(inner) if Some(name.as_str()) == input_arg => {
                return JsonValue::Object(inner);
            }
            other => {
                fields.insert(name.to_string(), other);
            }
        }
    }
    JsonValue::Object(fields)
}

/// Turn a field's arguments into a store query.
///
/// Pagination and ordering arguments are recognised by name. Every *other*
/// argument whose name matches a field on the target entity becomes an
/// equality filter — a heuristic that earns its place only because it is
/// reportable: an argument that matches nothing is ignored rather than
/// silently changing the answer.
fn selection_from_args(
    ctx: &ResolverContext<'_>,
    store: &Arc<EntityStore>,
    entity: &str,
) -> Selection {
    let mut selection = Selection::new();
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    let mut after: Option<Cursor> = None;
    let mut before: Option<Cursor> = None;
    let mut offset: Option<usize> = None;
    let mut limit: Option<usize> = None;
    let mut page_number: Option<usize> = None;

    let entity_fields: Vec<String> = store
        .graph()
        .get(entity)
        .map(|def| def.fields.iter().map(|f| f.name.to_string()).collect())
        .unwrap_or_default();

    for (name, accessor) in ctx.args.iter() {
        let value = accessor.as_value();
        match name.as_str() {
            "first" => first = as_usize(value),
            "last" => last = as_usize(value),
            "after" => after = as_string(value).map(Cursor::new),
            "before" => before = as_string(value).map(Cursor::new),
            "offset" | "skip" => offset = as_usize(value),
            "limit" => limit = as_usize(value),
            "page" => page_number = as_usize(value),
            "orderBy" | "sort" | "sortBy" => {
                if let Some(key) = sort_key(value) {
                    selection = selection.sorted_by(key);
                }
            }
            other => {
                if entity_fields.iter().any(|f| f == other) {
                    selection = selection.filter(Predicate::eq(other, to_json(value)));
                }
            }
        }
    }

    let page = if let Some(first) = first {
        Page::After {
            cursor: after,
            first,
        }
    } else if let Some(last) = last {
        Page::Before {
            cursor: before,
            last,
        }
    } else if let Some(limit) = limit {
        let skip = page_number
            .map(|p| p.saturating_sub(1).saturating_mul(limit))
            .or(offset)
            .unwrap_or(0);
        Page::Offset { skip, take: limit }
    } else if let Some(skip) = offset {
        Page::Offset {
            skip,
            take: DEFAULT_PAGE_SIZE,
        }
    } else {
        Page::All
    };

    selection.paged(page)
}

/// `-createdAt` and `{ field, direction }` are the two spellings in the wild.
fn sort_key(value: &GqlValue) -> Option<SortKey> {
    match value {
        GqlValue::String(_) | GqlValue::Enum(_) => {
            let raw = match value {
                GqlValue::String(s) => s.clone(),
                GqlValue::Enum(name) => name.to_string(),
                _ => return None,
            };
            if raw.is_empty() {
                return None;
            }
            Some(
                raw.strip_prefix('-')
                    .map_or_else(|| SortKey::asc(raw.as_str()), SortKey::desc),
            )
        }
        GqlValue::Object(fields) => {
            let field = fields.get("field").and_then(|v| match v {
                GqlValue::String(s) => Some(s.clone()),
                GqlValue::Enum(n) => Some(n.to_string()),
                _ => None,
            })?;
            let descending = fields.get("direction").is_some_and(|v| match v {
                GqlValue::String(s) => s.eq_ignore_ascii_case("desc"),
                GqlValue::Enum(n) => n.as_str().eq_ignore_ascii_case("desc"),
                _ => false,
            });
            Some(if descending {
                SortKey::desc(field)
            } else {
                SortKey::asc(field)
            })
        }
        _ => None,
    }
}

fn as_usize(value: &GqlValue) -> Option<usize> {
    match value {
        GqlValue::Number(n) => n.as_u64().and_then(|v| usize::try_from(v).ok()),
        GqlValue::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_string(value: &GqlValue) -> Option<String> {
    match value {
        GqlValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod cov_tests {
    use super::*;
    #[test]
    fn scratch_interface_covariance() {
        let sdl = "
            interface EdgeInterface { cursor: String! }
            interface ConnectionInterface { edges: [EdgeInterface]! }
            type AEdge implements EdgeInterface { cursor: String!, node: Thing }
            type AConn implements ConnectionInterface { edges: [AEdge!]! }
            type Thing { id: ID! }
            type Query { conn: AConn }
        ";
        let parsed = crate::spec::infer::graphql::parse_sdl(sdl).unwrap();
        let graph = crate::spec::infer::graphql::to_entity_graph(&parsed);
        let world = std::sync::Arc::new(World::new());
        world.add_entities(&graph).unwrap();
        match GraphQLBackend::build(&parsed, world) {
            Ok(_) => println!("BUILD OK"),
            Err(e) => println!("BUILD FAILED: {e}"),
        }
    }
}
