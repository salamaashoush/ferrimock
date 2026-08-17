//! A GraphQL schema is already an entity graph; this reads it off.
//!
//! Nothing here is inferred in the OpenAPI sense — the schema declares which
//! types exist, which fields link them, and how many of each a link yields.
//! The only judgement calls are which object types have identity, and which
//! ones are the Relay connection machinery around a link rather than a thing
//! in their own right.

use lean_string::LeanString;
use rustc_hash::FxHashSet;

use crate::core::world::model::{
    Cardinality, Carrier, CompositeKey, Confidence, ConnectionShape, EntityGraph, EntityType,
    FieldDef, Provenance, Relation, Rule, Scalar, ScalarKind, ValueSpec,
};
use crate::graphql::introspection::{ParsedSchema, TypeDefinition, TypeKind, TypeRef};
use crate::spec::infer::descriptions::{DescriptionHint, hint};
use crate::spec::infer::semantics::{semantic_of, text_shape_of};

/// How deep value objects are inlined before the expansion stops.
///
/// Entities are not inlined — they are links the store resolves on demand, so
/// depth there is the client's business. This bounds *value* objects, which
/// have no identity to stop at and which real schemas nest into each other in
/// cycles.
const MAX_EMBED_DEPTH: usize = 4;

/// The four fields the Relay spec gives `PageInfo`.
const PAGE_INFO_FIELDS: [&str; 4] = ["hasNextPage", "hasPreviousPage", "startCursor", "endCursor"];

/// What a schema says about itself, computed once and reused.
///
/// Which types have identity and which are connection machinery decides how
/// every field is read, so it is worked out once rather than per field.
pub struct SchemaFacts<'a> {
    entities: FxHashSet<&'a str>,
    connections: Connections,
}

impl<'a> SchemaFacts<'a> {
    #[must_use]
    pub fn of(schema: &'a ParsedSchema) -> Self {
        let roots = root_names(schema);
        let connections = connection_types(schema);
        let entities = schema
            .types
            .values()
            .filter(|def| is_entity(def, &roots, &connections))
            .map(|def| def.name.as_str())
            .collect();
        Self {
            entities,
            connections,
        }
    }
}

/// The shape of a value of `type_ref`, for a field nothing else could be
/// inferred about. This is what lets the bottom rung answer from the declared
/// type rather than returning nothing.
#[must_use]
pub fn value_spec_of(
    schema: &ParsedSchema,
    facts: &SchemaFacts<'_>,
    type_ref: &TypeRef,
    field_name: &str,
) -> ValueSpec {
    value_spec(
        type_ref,
        field_name,
        None,
        schema,
        &facts.entities,
        &facts.connections,
        "",
        &mut Expansion::new(),
    )
}

/// Compile a parsed schema into an entity graph.
pub fn to_entity_graph(schema: &ParsedSchema) -> EntityGraph {
    let facts = SchemaFacts::of(schema);
    let SchemaFacts {
        entities: entity_names,
        connections,
    } = facts;

    let mut graph = EntityGraph::new();
    // Sorted so the graph is built in the same order every run regardless of
    // hash-map iteration order; seeding depends on it.
    let mut definitions: Vec<&TypeDefinition> = schema
        .types
        .values()
        .filter(|def| entity_names.contains(def.name.as_str()))
        .collect();
    definitions.sort_by(|a, b| a.name.cmp(&b.name));

    for definition in definitions {
        graph.insert(entity_from(definition, schema, &entity_names, &connections));
    }
    graph
}

fn root_names(schema: &ParsedSchema) -> FxHashSet<&str> {
    [
        schema.query_type.as_deref(),
        schema.mutation_type.as_deref(),
        schema.subscription_type.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// An object type has identity when it carries an `ID`-typed key field, or
/// implements `Node`. Everything else is a value the owner inlines.
fn is_entity(
    definition: &TypeDefinition,
    roots: &FxHashSet<&str>,
    connections: &Connections,
) -> bool {
    if definition.kind != TypeKind::Object
        || definition.name.starts_with("__")
        || roots.contains(definition.name.as_str())
        || connections.is_machinery(&definition.name)
    {
        return false;
    }
    key_field(definition).is_some()
}

/// The field addressing an instance: an explicit `id`, else the first
/// `ID`-typed field, which is how schemas that spell it `sid` or `slug` are
/// still addressable.
fn key_field(definition: &TypeDefinition) -> Option<&str> {
    let implements_node = definition.interfaces.iter().any(|i| i.name() == "Node");

    if let Some(field) = definition.fields.iter().find(|f| f.name == "id") {
        return Some(field.name.as_str());
    }
    let id_typed = definition
        .fields
        .iter()
        .find(|f| f.field_type.name() == "ID")
        .map(|f| f.name.as_str());
    if implements_node {
        // `implements Node` promises an id even if it is spelled unusually.
        return id_typed.or(Some("id"));
    }
    id_typed
}

/// What is currently being inlined, so a cycle among value objects ends.
struct Expansion<'a> {
    open: Vec<&'a str>,
}

impl Expansion<'_> {
    fn new() -> Self {
        Self { open: Vec::new() }
    }

    fn would_cycle(&self, name: &str) -> bool {
        self.open.contains(&name)
    }

    fn depth(&self) -> usize {
        self.open.len()
    }
}

fn entity_from(
    definition: &TypeDefinition,
    schema: &ParsedSchema,
    entities: &FxHashSet<&str>,
    connections: &Connections,
) -> EntityType {
    let key = key_field(definition).unwrap_or("id");
    let mut entity = EntityType::new(
        definition.name.as_str(),
        CompositeKey::single(key),
        Provenance::new(Rule::GraphQLSchema, definition.name.as_str()),
    );
    entity.typename = Some(LeanString::from(definition.name.as_str()));

    for field in &definition.fields {
        let value = value_spec(
            &field.field_type,
            &field.name,
            field.description.as_deref(),
            schema,
            entities,
            connections,
            &definition.name,
            &mut Expansion::new(),
        );

        // A *value* field taking arguments is a query, not something stored:
        // two calls with different arguments cannot both be "the" value of one
        // field. A *link* is different — its arguments are how it is read, and
        // dropping it loses the relation entirely.
        if value.relation().is_none() && !field.args.is_empty() && !is_pagination_args(field) {
            continue;
        }

        entity = entity.with_field(FieldDef::new(
            field.name.as_str(),
            value,
            !field.field_type.is_non_null(),
        ));
    }

    entity
}

/// Pagination arguments do not stop a field from being a relation — they are
/// how the relation is read.
fn is_pagination_args(field: &crate::graphql::introspection::FieldDefinition) -> bool {
    field.args.iter().all(|arg| {
        matches!(
            arg.name.as_str(),
            "first" | "last" | "after" | "before" | "offset" | "limit" | "orderBy" | "sort"
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn value_spec<'a>(
    type_ref: &TypeRef,
    field_name: &str,
    description: Option<&str>,
    schema: &'a ParsedSchema,
    entities: &FxHashSet<&str>,
    connections: &Connections,
    owner: &str,
    expansion: &mut Expansion<'a>,
) -> ValueSpec {
    let named = type_ref.name();

    if let Some(shape) = connections.shape_of(named) {
        let node = shape.node_type.as_str();
        let members = concrete_members(schema, entities, node);
        if entities.contains(node) || !members.is_empty() {
            let relation = Relation::new(
                node,
                Cardinality::Many,
                Carrier::Connection(ConnectionShape {
                    connection_type: LeanString::from(named),
                    edge_type: LeanString::from(shape.edge_type.as_str()),
                    page_info_type: LeanString::from(shape.page_info_type.as_str()),
                }),
                Confidence::STRUCTURAL,
                Provenance::new(Rule::RelayConnection, format!("{owner}.{field_name}")),
            );
            return ValueSpec::Relation(Box::new(relation.abstract_target(members)));
        }
    }

    // An interface or union whose members have identity is still a link — the
    // one it resolves to is just chosen per instance. Polymorphic collections
    // are ordinary in real schemas, and skipping them leaves a hole.
    let members = concrete_members(schema, entities, named);
    if entities.contains(named) || !members.is_empty() {
        let relation = Relation::new(
            named,
            if type_ref.is_list() {
                Cardinality::Many
            } else {
                Cardinality::One
            },
            Carrier::Embedded,
            Confidence::STRUCTURAL,
            Provenance::new(Rule::GraphQLSchema, format!("{owner}.{field_name}")),
        )
        .abstract_target(members);
        let spec = ValueSpec::Relation(Box::new(relation));
        return if type_ref.is_list() {
            ValueSpec::List(Box::new(spec))
        } else {
            spec
        };
    }

    let inner = match schema.types.get(named).map(|def| &def.kind) {
        Some(TypeKind::Enum) => enum_spec(schema, named),
        Some(TypeKind::Object | TypeKind::Interface) => {
            embedded_spec(schema, named, entities, connections, expansion)
        }
        // A union of value objects has no single shape to inline; the first
        // member is a truthful sample of one of them.
        Some(TypeKind::Union) => schema
            .types
            .get(named)
            .and_then(|def| def.possible_types.first())
            .map_or_else(
                || scalar_spec(named, field_name, description),
                |member| embedded_spec(schema, member.name(), entities, connections, expansion),
            ),
        _ => scalar_spec(named, field_name, description),
    };

    if type_ref.is_list() {
        ValueSpec::List(Box::new(inner))
    } else {
        inner
    }
}

/// The entities an interface or union can actually be, for a caller that
/// already has the schema's facts.
#[must_use]
pub fn members_of(schema: &ParsedSchema, facts: &SchemaFacts<'_>, name: &str) -> Vec<LeanString> {
    concrete_members(schema, &facts.entities, name)
}

/// Whether a named type is an entity.
#[must_use]
pub fn is_entity_name(facts: &SchemaFacts<'_>, name: &str) -> bool {
    facts.entities.contains(name)
}

/// The entities an interface or union can actually be.
///
/// Empty for a concrete type, and empty for an abstract type none of whose
/// members have identity — which keeps it a value object rather than turning
/// it into a link to nothing.
fn concrete_members(
    schema: &ParsedSchema,
    entities: &FxHashSet<&str>,
    name: &str,
) -> Vec<LeanString> {
    let Some(definition) = schema.types.get(name) else {
        return Vec::new();
    };

    let mut members: Vec<LeanString> = match definition.kind {
        TypeKind::Union => definition
            .possible_types
            .iter()
            .map(TypeRef::name)
            .filter(|member| entities.contains(member))
            .map(LeanString::from)
            .collect(),
        TypeKind::Interface => schema
            .types
            .values()
            .filter(|candidate| {
                candidate.interfaces.iter().any(|i| i.name() == name)
                    && entities.contains(candidate.name.as_str())
            })
            .map(|candidate| LeanString::from(candidate.name.as_str()))
            .collect(),
        _ => Vec::new(),
    };

    // Sorted so a member is picked the same way on every run.
    members.sort();
    members
}

fn enum_spec(schema: &ParsedSchema, name: &str) -> ValueSpec {
    let options = schema.types.get(name).map_or_else(Vec::new, |def| {
        def.enum_values
            .iter()
            .map(|v| LeanString::from(v.name.as_str()))
            .collect()
    });
    if options.is_empty() {
        ValueSpec::Scalar(Scalar::new(ScalarKind::String))
    } else {
        ValueSpec::Enum(options)
    }
}

fn embedded_spec<'a>(
    schema: &'a ParsedSchema,
    name: &str,
    entities: &FxHashSet<&str>,
    connections: &Connections,
    expansion: &mut Expansion<'a>,
) -> ValueSpec {
    let Some(definition) = schema.types.get(name) else {
        return ValueSpec::Scalar(Scalar::new(ScalarKind::String));
    };

    // A value object has no identity to stop the expansion at, so two of them
    // referring to each other inline forever. Real schemas do this: it is what
    // turned a 597KB production schema into a stack overflow.
    if expansion.would_cycle(&definition.name) || expansion.depth() >= MAX_EMBED_DEPTH {
        return ValueSpec::Embedded(Vec::new());
    }

    expansion.open.push(definition.name.as_str());
    let fields = definition
        .fields
        .iter()
        .filter(|field| field.args.is_empty())
        .map(|field| {
            FieldDef::new(
                field.name.as_str(),
                value_spec(
                    &field.field_type,
                    &field.name,
                    field.description.as_deref(),
                    schema,
                    entities,
                    connections,
                    name,
                    expansion,
                ),
                !field.field_type.is_non_null(),
            )
        })
        .collect();
    expansion.open.pop();

    ValueSpec::Embedded(fields)
}

fn scalar_spec(type_name: &str, field_name: &str, description: Option<&str>) -> ValueSpec {
    // The description is the only place a schema-only pipeline can learn a
    // domain vocabulary, so it is consulted before anything is guessed from
    // the name.
    let mined = description.and_then(hint);

    // A stated value is the strongest evidence there is: the description is
    // telling you the answer rather than describing it.
    match mined {
        Some(DescriptionHint::Constant(value)) => {
            return ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(
                crate::type_detector::FieldType::Constant(serde_json::Value::String(
                    value.to_string(),
                )),
            ));
        }
        Some(DescriptionHint::OneOf(values)) => return ValueSpec::Enum(values),
        _ => {}
    }

    let kind = scalar_kind(type_name);
    let mut scalar = Scalar::new(kind).with_shape(text_shape_of(field_name));

    // The field's own name and declared type beat prose about it. A field
    // named `id` on an `ID` is an identifier however its description rambles.
    if let Some(field_type) = semantic_of(field_name, type_name, None) {
        scalar = scalar.with_semantic(field_type);
    } else if let Some(DescriptionHint::Semantic(field_type)) = mined {
        scalar = scalar.with_semantic(field_type);
    }
    ValueSpec::Scalar(scalar)
}

fn scalar_kind(type_name: &str) -> ScalarKind {
    match type_name {
        "Int" => ScalarKind::Int,
        "Float" => ScalarKind::Float,
        "Boolean" => ScalarKind::Boolean,
        "ID" => ScalarKind::Id,
        "String" => ScalarKind::String,
        other => ScalarKind::Custom(LeanString::from(other)),
    }
}

/// The Relay connection types in a schema, recognised by shape, not name.
///
/// A type called `Connection` that is not one must not be treated as one, and
/// a correctly shaped type named something else must be.
#[derive(Debug, Default)]
pub struct Connections {
    shapes: Vec<DetectedConnection>,
    machinery: FxHashSet<String>,
}

#[derive(Debug, Clone)]
pub struct DetectedConnection {
    pub connection_type: String,
    pub edge_type: String,
    pub node_type: String,
    pub page_info_type: String,
}

impl Connections {
    #[must_use]
    pub fn shape_of(&self, name: &str) -> Option<&DetectedConnection> {
        self.shapes.iter().find(|s| s.connection_type == name)
    }

    /// Types that exist only to carry a connection, and are therefore not
    /// entities in their own right.
    #[must_use]
    pub fn is_machinery(&self, name: &str) -> bool {
        self.machinery.contains(name)
    }

    pub fn detected(&self) -> impl Iterator<Item = &DetectedConnection> {
        self.shapes.iter()
    }
}

/// Find every type matching the Relay connection shape.
pub fn connection_types(schema: &ParsedSchema) -> Connections {
    let mut shapes = Vec::new();
    let mut machinery = FxHashSet::default();

    for definition in schema.types.values() {
        if definition.kind != TypeKind::Object {
            continue;
        }
        let Some(edges) = definition.fields.iter().find(|f| f.name == "edges") else {
            continue;
        };
        let Some(page_info) = definition.fields.iter().find(|f| f.name == "pageInfo") else {
            continue;
        };
        if !edges.field_type.is_list() {
            continue;
        }

        let edge_name = edges.field_type.name();
        let Some(edge) = schema.types.get(edge_name) else {
            continue;
        };
        let Some(node) = edge.fields.iter().find(|f| f.name == "node") else {
            continue;
        };
        if !edge.fields.iter().any(|f| f.name == "cursor") {
            continue;
        }

        let page_info_name = page_info.field_type.name();
        let Some(page_info_def) = schema.types.get(page_info_name) else {
            continue;
        };
        let has_page_info_fields = PAGE_INFO_FIELDS
            .iter()
            .all(|wanted| page_info_def.fields.iter().any(|f| f.name == *wanted));
        if !has_page_info_fields {
            continue;
        }

        machinery.insert(definition.name.clone());
        machinery.insert(edge_name.to_string());
        machinery.insert(page_info_name.to_string());
        shapes.push(DetectedConnection {
            connection_type: definition.name.clone(),
            edge_type: edge_name.to_string(),
            node_type: node.field_type.name().to_string(),
            page_info_type: page_info_name.to_string(),
        });
    }

    shapes.sort_by(|a, b| a.connection_type.cmp(&b.connection_type));
    Connections { shapes, machinery }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::get_unwrap,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::spec::infer::graphql::sdl::parse_sdl;

    const BLOG: &str = r"
        interface Node { id: ID! }

        type User implements Node {
          id: ID!
          name: String!
          email: String
          address: Address
          posts: [Post!]!
        }

        type Address { city: String!, zip: String! }

        type Post implements Node {
          id: ID!
          title: String!
          status: Status!
          author: User!
          viewsPerDay: [Int!]
        }

        enum Status { DRAFT PUBLISHED }

        type Query {
          user(id: ID!): User
          posts: [Post!]!
        }
    ";

    fn blog() -> EntityGraph {
        to_entity_graph(&parse_sdl(BLOG).unwrap())
    }

    #[test]
    fn identified_object_types_become_entities() {
        let graph = blog();
        assert!(graph.contains("User"));
        assert!(graph.contains("Post"));
        assert_eq!(graph.len(), 2);
    }

    #[test]
    fn roots_and_value_objects_are_not_entities() {
        let graph = blog();
        assert!(!graph.contains("Query"));
        assert!(
            !graph.contains("Address"),
            "a type with no identity is a value, not an entity"
        );
    }

    #[test]
    fn a_value_object_is_inlined_on_its_owner() {
        let graph = blog();
        let address = graph.get("User").unwrap().field("address").unwrap();
        assert!(matches!(address.value, ValueSpec::Embedded(_)));
    }

    #[test]
    fn relations_carry_cardinality_from_the_wrapper() {
        let graph = blog();
        let posts = graph.get("User").unwrap().field("posts").unwrap();
        assert_eq!(posts.relation().unwrap().cardinality, Cardinality::Many);
        let author = graph.get("Post").unwrap().field("author").unwrap();
        assert_eq!(author.relation().unwrap().cardinality, Cardinality::One);
    }

    #[test]
    fn nullability_comes_from_the_wrapper() {
        let graph = blog();
        let user = graph.get("User").unwrap();
        assert!(!user.field("name").unwrap().nullable);
        assert!(user.field("email").unwrap().nullable);
    }

    #[test]
    fn enums_keep_their_declared_options() {
        let graph = blog();
        let status = graph.get("Post").unwrap().field("status").unwrap();
        let ValueSpec::Enum(options) = &status.value else {
            panic!("status should be an enum")
        };
        assert_eq!(options.len(), 2);
    }

    #[test]
    fn scalar_lists_stay_lists_of_scalars() {
        let graph = blog();
        let views = graph.get("Post").unwrap().field("viewsPerDay").unwrap();
        assert!(views.value.is_list());
        assert!(views.relation().is_none());
    }

    /// A description that merely *mentions* a URL does not make an `ID!` a
    /// URL. `Folder.id` reads "...for the URL https://app.example.com/folders/123
    /// the folder_id is 123", which made its keys stop looking like the keys
    /// of every sibling entity.
    #[test]
    fn prose_does_not_override_the_declared_identifier() {
        let schema = parse_sdl(
            "type Folder {\n\
               \"\"\"\n\
               The unique identifier of a folder.\n\
               The ID can be found by visiting a folder and copying it from the URL, \
               e.g. for https://app.example.com/folders/123 the folder id is 123.\n\
               \"\"\"\n\
               id: ID!\n\
               name: String!\n\
             }\n\
             type Query { folder: Folder }",
        )
        .unwrap();

        let graph = to_entity_graph(&schema);
        let id = graph.get("Folder").unwrap().field("id").unwrap();
        let ValueSpec::Scalar(scalar) = &id.value else {
            panic!("id should stay a scalar, got {:?}", id.value)
        };
        assert!(
            matches!(scalar.semantic, Some(crate::type_detector::FieldType::Uuid)),
            "an `ID!` named `id` is an identifier whatever its prose says, got {:?}",
            scalar.semantic
        );
    }

    #[test]
    fn a_description_still_supplies_what_the_name_cannot() {
        let schema = parse_sdl(
            "type Part {\n\
               id: ID!\n\
               \"The MIME type of the content (e.g., \\\"text/plain\\\", \\\"image/png\\\").\"\n\
               mediaType: String\n\
             }\n\
             type Query { part: Part }",
        )
        .unwrap();

        let graph = to_entity_graph(&schema);
        let media = graph.get("Part").unwrap().field("mediaType").unwrap();
        let ValueSpec::Enum(values) = &media.value else {
            panic!(
                "mediaType should take its vocabulary from the description, got {:?}",
                media.value
            )
        };
        assert_eq!(values, &["text/plain", "image/png"]);
    }

    #[test]
    fn field_names_drive_semantic_detection() {
        let graph = blog();
        let email = graph.get("User").unwrap().field("email").unwrap();
        let ValueSpec::Scalar(scalar) = &email.value else {
            panic!("email should be a scalar")
        };
        assert!(
            scalar.semantic.is_some(),
            "a field named `email` should be detected as one"
        );
    }

    #[test]
    fn a_relay_connection_is_recognised_by_shape() {
        let schema = parse_sdl(
            r"
            type User { id: ID!, friends: FriendConnection }
            type FriendConnection { edges: [FriendEdge], pageInfo: PageInfo! }
            type FriendEdge { node: User, cursor: String! }
            type PageInfo {
              hasNextPage: Boolean!
              hasPreviousPage: Boolean!
              startCursor: String
              endCursor: String
            }
            type Query { users: [User!]! }
        ",
        )
        .unwrap();

        let graph = to_entity_graph(&schema);
        assert_eq!(graph.len(), 1, "only User has identity");

        let friends = graph.get("User").unwrap().field("friends").unwrap();
        let relation = friends.relation().expect("friends should be a relation");
        assert_eq!(relation.target.as_str(), "User");
        assert_eq!(relation.cardinality, Cardinality::Many);
        let Carrier::Connection(shape) = &relation.carrier else {
            panic!("friends should be carried by a connection")
        };
        assert_eq!(shape.edge_type.as_str(), "FriendEdge");
    }

    #[test]
    fn a_type_merely_named_connection_is_not_one() {
        let schema = parse_sdl(
            "type User { id: ID!, c: WeirdConnection } type WeirdConnection { id: ID!, note: String } type Query { u: User }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        assert!(
            graph.contains("WeirdConnection"),
            "a wrongly shaped type must be treated as an ordinary entity, not machinery"
        );
        let field = graph.get("User").unwrap().field("c").unwrap();
        assert!(matches!(
            field.relation().unwrap().carrier,
            Carrier::Embedded
        ));
    }

    #[test]
    fn a_field_taking_real_arguments_is_not_a_stored_value() {
        let schema = parse_sdl(
            "type User { id: ID!, search(term: String!): [String!] } type Query { u: User }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        assert!(graph.get("User").unwrap().field("search").is_none());
    }

    #[test]
    fn a_relation_with_only_pagination_arguments_survives() {
        let schema = parse_sdl(
            "type User { id: ID!, posts(first: Int, after: String): [Post!] } type Post { id: ID! } type Query { u: User }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let posts = graph.get("User").unwrap().field("posts").unwrap();
        assert_eq!(posts.relation().unwrap().target.as_str(), "Post");
    }

    #[test]
    fn compiling_is_order_independent() {
        let schema = parse_sdl(BLOG).unwrap();
        let names_a: Vec<_> = to_entity_graph(&schema)
            .entities()
            .map(|e| e.name.to_string())
            .collect();
        let names_b: Vec<_> = to_entity_graph(&schema)
            .entities()
            .map(|e| e.name.to_string())
            .collect();
        assert_eq!(names_a, names_b);
    }

    /// Value objects have no identity to stop an expansion at, so a cycle
    /// among them inlines forever. A production schema of 1833 types did
    /// exactly this and overflowed the stack.
    #[test]
    fn mutually_recursive_value_objects_terminate() {
        let schema = parse_sdl(
            "type A { b: B, label: String! } type B { a: A, note: String! } \
             type Item { id: ID!, a: A } type Query { i: Item }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);

        let a = graph.get("Item").unwrap().field("a").unwrap();
        let ValueSpec::Embedded(a_fields) = &a.value else {
            panic!("a should be embedded")
        };
        assert!(
            a_fields.iter().any(|f| f.name == "label"),
            "the first level should still expand"
        );
    }

    #[test]
    fn a_self_referencing_value_object_stops_expanding() {
        let schema = parse_sdl(
            "type Tree { child: Tree, label: String! } type Item { id: ID!, tree: Tree } type Query { i: Item }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let tree = graph.get("Item").unwrap().field("tree").unwrap();
        let ValueSpec::Embedded(fields) = &tree.value else {
            panic!("tree should be embedded")
        };
        let child = fields.iter().find(|f| f.name == "child").unwrap();
        assert!(
            matches!(&child.value, ValueSpec::Embedded(inner) if inner.is_empty()),
            "the cycle stops rather than the field vanishing"
        );
        assert!(fields.iter().any(|f| f.name == "label"));
    }

    #[test]
    fn a_deep_chain_of_value_objects_is_bounded() {
        let schema = parse_sdl(
            "type L1 { n: L2 } type L2 { n: L3 } type L3 { n: L4 } type L4 { n: L5 } \
             type L5 { n: L6 } type L6 { leaf: String! } \
             type Item { id: ID!, chain: L1 } type Query { i: Item }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let mut value = &graph.get("Item").unwrap().field("chain").unwrap().value;
        let mut depth = 0;
        while let ValueSpec::Embedded(fields) = value {
            let Some(next) = fields.first() else { break };
            value = &next.value;
            depth += 1;
            assert!(depth < 20, "expansion should be bounded");
        }
        assert!(
            depth <= MAX_EMBED_DEPTH + 1,
            "depth {depth} exceeded the cap"
        );
    }
}
