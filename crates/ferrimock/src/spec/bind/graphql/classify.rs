//! Deciding what a root field *does*, at build time.
//!
//! Entity fields are genuinely generic — a parent record and a field name are
//! all a resolver needs. Root fields are not: `posts(first:)`, `user(id:)` and
//! `searchPostsByTag(tag:, sort:)` have nothing in common but their position.
//! So each one is classified once, into a rung, and the rung it landed on is
//! reportable. The bottom rung invents data; a mock that does that for half a
//! schema must not look like one that does not.

use lean_string::LeanString;

use crate::graphql::introspection::{FieldDefinition, ParsedSchema, TypeDefinition, TypeKind};
use crate::spec::infer::graphql::entities::{SchemaFacts, members_of};
use crate::spec::model::{ConnectionShape, EntityGraph};

/// Argument names that describe *how* to read a list rather than *which* one.
const PAGINATION_ARGS: [&str; 8] = [
    "first", "last", "after", "before", "offset", "limit", "skip", "page",
];
const ORDER_ARGS: [&str; 3] = ["orderBy", "sort", "sortBy"];

/// What a root field resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootPlan {
    /// One instance, addressed by key.
    Get {
        entity: LeanString,
        /// Concrete entities behind an interface or union return type.
        members: Vec<LeanString>,
        key_arg: LeanString,
    },
    /// Many instances, optionally wrapped in a Relay connection.
    List {
        entity: LeanString,
        /// Concrete entities behind an interface or union return type.
        members: Vec<LeanString>,
        connection: Option<ConnectionShape>,
        /// The field on the payload holding the entities, when the list is
        /// wrapped in a result object rather than returned directly.
        payload_field: Option<LeanString>,
    },
    Create {
        entity: LeanString,
        input_arg: Option<LeanString>,
        payload_field: Option<LeanString>,
    },
    Update {
        entity: LeanString,
        key_arg: LeanString,
        input_arg: Option<LeanString>,
        payload_field: Option<LeanString>,
    },
    Delete {
        entity: LeanString,
        key_arg: LeanString,
        payload_field: Option<LeanString>,
    },
    /// Nothing about the field says what it does. Answered from its declared
    /// return type, stably, and counted.
    Unclassified,
}

impl RootPlan {
    #[must_use]
    pub fn rung(&self) -> &'static str {
        match self {
            RootPlan::Get { .. } => "get",
            RootPlan::List { .. } => "list",
            RootPlan::Create { .. } => "create",
            RootPlan::Update { .. } => "update",
            RootPlan::Delete { .. } => "delete",
            RootPlan::Unclassified => "unclassified",
        }
    }

    #[must_use]
    pub fn is_classified(&self) -> bool {
        !matches!(self, RootPlan::Unclassified)
    }
}

/// Whether a root field lives on `Query` or `Mutation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    Query,
    Mutation,
}

/// Classify one root field.
#[must_use]
pub fn classify(
    field: &FieldDefinition,
    kind: RootKind,
    schema: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
) -> RootPlan {
    let Some(target) = payload_target(field, schema, graph, facts) else {
        return RootPlan::Unclassified;
    };

    match kind {
        RootKind::Query => classify_query(field, &target, graph),
        RootKind::Mutation => classify_mutation(field, &target, graph),
    }
}

/// What a root field ultimately yields: which entity, whether it is a list,
/// and the payload field it had to be dug out of.
#[derive(Debug, Clone)]
struct Target {
    entity: LeanString,
    members: Vec<LeanString>,
    is_list: bool,
    connection: Option<ConnectionShape>,
    payload_field: Option<LeanString>,
}

fn payload_target(
    field: &FieldDefinition,
    schema: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
) -> Option<Target> {
    let named = field.field_type.name();

    // An interface or union return type is still addressable — the concrete
    // type is chosen per instance. `Query.item(id:)` returning a union of
    // File | Folder | Weblink is the ordinary shape, not an exception.
    let members = members_of(schema, facts, named);
    if graph.contains(named) || !members.is_empty() {
        return Some(Target {
            entity: LeanString::from(named),
            members,
            is_list: field.field_type.is_list(),
            connection: None,
            payload_field: None,
        });
    }

    let definition = schema.types.get(named)?;
    if definition.kind != TypeKind::Object {
        return None;
    }

    if let Some(connection) = connection_target(definition, schema, graph, facts) {
        return Some(connection);
    }

    // A mutation payload (`CreatePostPayload { post, errors }`) wraps the
    // thing it produced. Exactly one entity-typed field means there is no
    // ambiguity about which one that is.
    let mut entity_fields = definition.fields.iter().filter(|f| {
        graph.contains(f.field_type.name())
            || !members_of(schema, facts, f.field_type.name()).is_empty()
    });
    let only = entity_fields.next()?;
    if entity_fields.next().is_some() {
        return None;
    }

    Some(Target {
        entity: LeanString::from(only.field_type.name()),
        members: members_of(schema, facts, only.field_type.name()),
        is_list: only.field_type.is_list(),
        connection: None,
        payload_field: Some(LeanString::from(only.name.as_str())),
    })
}

fn connection_target(
    definition: &TypeDefinition,
    schema: &ParsedSchema,
    graph: &EntityGraph,
    facts: &SchemaFacts<'_>,
) -> Option<Target> {
    let edges = definition.fields.iter().find(|f| f.name == "edges")?;
    let page_info = definition.fields.iter().find(|f| f.name == "pageInfo")?;
    let edge = schema.types.get(edges.field_type.name())?;
    let node = edge.fields.iter().find(|f| f.name == "node")?;
    if !edge.fields.iter().any(|f| f.name == "cursor") {
        return None;
    }
    let entity = node.field_type.name();
    let members = members_of(schema, facts, entity);
    if !graph.contains(entity) && members.is_empty() {
        return None;
    }

    Some(Target {
        entity: LeanString::from(entity),
        members,
        is_list: true,
        connection: Some(ConnectionShape {
            connection_type: LeanString::from(definition.name.as_str()),
            edge_type: LeanString::from(edge.name.as_str()),
            page_info_type: LeanString::from(page_info.field_type.name()),
        }),
        payload_field: None,
    })
}

fn classify_query(field: &FieldDefinition, target: &Target, graph: &EntityGraph) -> RootPlan {
    if target.is_list {
        return RootPlan::List {
            entity: target.entity.clone(),
            members: target.members.clone(),
            connection: target.connection.clone(),
            payload_field: target.payload_field.clone(),
        };
    }

    match key_argument(field, target, graph) {
        Some(key_arg) => RootPlan::Get {
            entity: target.entity.clone(),
            members: target.members.clone(),
            key_arg,
        },
        // A single entity with no way to say *which* one is not a lookup; it
        // is something like `viewer`, answered from the store all the same but
        // without pretending the argument list said so.
        None if field.args.is_empty() => RootPlan::Get {
            entity: target.entity.clone(),
            members: target.members.clone(),
            key_arg: LeanString::new(),
        },
        None => RootPlan::Unclassified,
    }
}

fn classify_mutation(field: &FieldDefinition, target: &Target, graph: &EntityGraph) -> RootPlan {
    let verb = leading_verb(&field.name);
    let key_arg = key_argument(field, target, graph);
    let input_arg = input_argument(field, graph, target);

    match verb {
        Verb::Create => RootPlan::Create {
            entity: target.entity.clone(),
            input_arg,
            payload_field: target.payload_field.clone(),
        },
        Verb::Update => key_arg.map_or(RootPlan::Unclassified, |key_arg| RootPlan::Update {
            entity: target.entity.clone(),
            key_arg,
            input_arg,
            payload_field: target.payload_field.clone(),
        }),
        Verb::Delete => key_arg.map_or(RootPlan::Unclassified, |key_arg| RootPlan::Delete {
            entity: target.entity.clone(),
            key_arg,
            payload_field: target.payload_field.clone(),
        }),
        Verb::Unknown => RootPlan::Unclassified,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Create,
    Update,
    Delete,
    Unknown,
}

/// The verb a mutation name starts with. Name prefixes are a convention, not a
/// rule, which is why an unrecognised one drops to the bottom rung rather than
/// guessing a write.
fn leading_verb(name: &str) -> Verb {
    const CREATE: [&str; 4] = ["create", "add", "new", "insert"];
    const UPDATE: [&str; 5] = ["update", "edit", "modify", "patch", "set"];
    const DELETE: [&str; 4] = ["delete", "remove", "destroy", "archive"];

    let lowered = name.to_ascii_lowercase();
    if CREATE.iter().any(|v| lowered.starts_with(v)) {
        Verb::Create
    } else if UPDATE.iter().any(|v| lowered.starts_with(v)) {
        Verb::Update
    } else if DELETE.iter().any(|v| lowered.starts_with(v)) {
        Verb::Delete
    } else {
        Verb::Unknown
    }
}

/// The argument naming which instance to act on: the entity's own key field,
/// `id`, or `<entity>Id`.
fn key_argument(
    field: &FieldDefinition,
    target: &Target,
    graph: &EntityGraph,
) -> Option<LeanString> {
    let key_field = graph
        .get(target.entity.as_str())
        .and_then(|entity| entity.key.as_single().cloned())
        .unwrap_or_else(|| LeanString::from("id"));

    let entity_id = format!("{}Id", lower_first(target.entity.as_str()));
    let candidates = [key_field.as_str(), "id", entity_id.as_str()];

    field
        .args
        .iter()
        .find(|arg| {
            candidates
                .iter()
                .any(|c| arg.name.eq_ignore_ascii_case(c))
        })
        .map(|arg| LeanString::from(arg.name.as_str()))
}

/// The argument carrying the values to write: an input object, or the first
/// argument that is neither the key nor pagination.
fn input_argument(
    field: &FieldDefinition,
    graph: &EntityGraph,
    target: &Target,
) -> Option<LeanString> {
    let key = key_argument(field, target, graph);
    field
        .args
        .iter()
        .find(|arg| {
            Some(arg.name.as_str()) != key.as_deref()
                && !PAGINATION_ARGS.contains(&arg.name.as_str())
                && !ORDER_ARGS.contains(&arg.name.as_str())
        })
        .map(|arg| LeanString::from(arg.name.as_str()))
}

fn lower_first(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_lowercase().collect::<String>() + chars.as_str()
    })
}

/// Whether an argument selects a slice rather than a subject.
#[must_use]
pub fn is_pagination_arg(name: &str) -> bool {
    PAGINATION_ARGS.contains(&name)
}

/// Whether an argument states an ordering.
#[must_use]
pub fn is_order_arg(name: &str) -> bool {
    ORDER_ARGS.contains(&name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::get_unwrap, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::spec::infer::graphql::{parse_sdl, to_entity_graph};

    const SCHEMA: &str = r"
        type User { id: ID!, name: String! }
        type Post { id: ID!, title: String!, status: String! }

        type PostConnection { edges: [PostEdge], pageInfo: PageInfo! }
        type PostEdge { node: Post, cursor: String! }
        type PageInfo {
          hasNextPage: Boolean!
          hasPreviousPage: Boolean!
          startCursor: String
          endCursor: String
        }

        input PostInput { title: String!, status: String }
        type CreatePostPayload { post: Post, errors: [String!]! }

        type Query {
          user(id: ID!): User
          viewer: User
          users(first: Int, after: String): [User!]!
          postFeed(first: Int): PostConnection
          searchPosts(term: String!, limit: Int): [Post!]!
        }

        type Mutation {
          createPost(input: PostInput!): CreatePostPayload
          updatePost(id: ID!, input: PostInput!): Post
          deletePost(id: ID!): Post
          publishPost(id: ID!): Post
        }
    ";

    fn plan(root: &str, field: &str) -> RootPlan {
        let schema = parse_sdl(SCHEMA).unwrap();
        let graph = to_entity_graph(&schema);
        let definition = schema.types.get(root).unwrap();
        let field = definition.fields.iter().find(|f| f.name == field).unwrap();
        let kind = if root == "Query" {
            RootKind::Query
        } else {
            RootKind::Mutation
        };
        let facts = SchemaFacts::of(&schema);
        classify(field, kind, &schema, &graph, &facts)
    }

    #[test]
    fn a_single_entity_with_a_key_argument_is_a_lookup() {
        let RootPlan::Get { entity, key_arg, .. } = plan("Query", "user") else {
            panic!("user should be a lookup")
        };
        assert_eq!(entity.as_str(), "User");
        assert_eq!(key_arg.as_str(), "id");
    }

    #[test]
    fn a_single_entity_with_no_arguments_still_resolves() {
        let RootPlan::Get { entity, key_arg, .. } = plan("Query", "viewer") else {
            panic!("viewer should resolve to an entity")
        };
        assert_eq!(entity.as_str(), "User");
        assert!(key_arg.is_empty(), "there is no key to read");
    }

    #[test]
    fn a_list_of_entities_is_a_list() {
        let RootPlan::List {
            entity, connection, ..
        } = plan("Query", "users")
        else {
            panic!("users should be a list")
        };
        assert_eq!(entity.as_str(), "User");
        assert!(connection.is_none());
    }

    #[test]
    fn a_connection_is_a_list_that_remembers_its_shape() {
        let RootPlan::List {
            entity, connection, ..
        } = plan("Query", "postFeed")
        else {
            panic!("postFeed should be a list")
        };
        assert_eq!(entity.as_str(), "Post");
        let shape = connection.expect("the connection shape should be kept");
        assert_eq!(shape.edge_type.as_str(), "PostEdge");
    }

    #[test]
    fn extra_arguments_do_not_stop_a_list_resolving() {
        // `term` becomes a filter attempt at resolve time; the field still
        // reads the store rather than inventing a list.
        let RootPlan::List { entity, .. } = plan("Query", "searchPosts") else {
            panic!("searchPosts should still be a list")
        };
        assert_eq!(entity.as_str(), "Post");
    }

    #[test]
    fn a_create_mutation_unwraps_its_payload() {
        let RootPlan::Create {
            entity,
            input_arg,
            payload_field,
        } = plan("Mutation", "createPost")
        else {
            panic!("createPost should create")
        };
        assert_eq!(entity.as_str(), "Post");
        assert_eq!(input_arg.as_deref(), Some("input"));
        assert_eq!(payload_field.as_deref(), Some("post"));
    }

    #[test]
    fn an_update_needs_a_key_and_finds_its_input() {
        let RootPlan::Update {
            entity,
            key_arg,
            input_arg,
            ..
        } = plan("Mutation", "updatePost")
        else {
            panic!("updatePost should update")
        };
        assert_eq!(entity.as_str(), "Post");
        assert_eq!(key_arg.as_str(), "id");
        assert_eq!(input_arg.as_deref(), Some("input"));
    }

    #[test]
    fn a_delete_is_recognised() {
        let RootPlan::Delete { entity, key_arg, .. } = plan("Mutation", "deletePost") else {
            panic!("deletePost should delete")
        };
        assert_eq!(entity.as_str(), "Post");
        assert_eq!(key_arg.as_str(), "id");
    }

    #[test]
    fn an_unrecognised_verb_drops_to_the_bottom_rung() {
        assert_eq!(plan("Mutation", "publishPost"), RootPlan::Unclassified);
        assert!(!RootPlan::Unclassified.is_classified());
    }

    #[test]
    fn a_field_returning_something_that_is_not_an_entity_is_unclassified() {
        let schema = parse_sdl("type Query { ping: String! } type User { id: ID! }").unwrap();
        let graph = to_entity_graph(&schema);
        let query = schema.types.get("Query").unwrap();
        let ping = query.fields.iter().find(|f| f.name == "ping").unwrap();
        assert_eq!(
            classify(ping, RootKind::Query, &schema, &graph, &SchemaFacts::of(&schema)),
            RootPlan::Unclassified
        );
    }

    #[test]
    fn an_ambiguous_payload_is_not_guessed_at() {
        let schema = parse_sdl(
            r"
            type User { id: ID! }
            type Post { id: ID! }
            type Both { user: User, post: Post }
            type Query { both: Both }
        ",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let query = schema.types.get("Query").unwrap();
        let both = query.fields.iter().find(|f| f.name == "both").unwrap();
        assert_eq!(
            classify(both, RootKind::Query, &schema, &graph, &SchemaFacts::of(&schema)),
            RootPlan::Unclassified,
            "two entity-typed fields give no single answer"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::get_unwrap)]
mod abstract_tests {
    use super::*;
    use crate::spec::infer::graphql::{parse_sdl, to_entity_graph};

    /// `Query.item(id:)` returning a union of concrete types is the ordinary
    /// shape of a content API, not an exception to be dropped.
    #[test]
    fn a_union_return_type_is_still_addressable() {
        let schema = parse_sdl(
            "type File { id: ID!, name: String! } type Folder { id: ID!, name: String! } \
             union Item = File | Folder \
             type Query { item(id: ID!): Item, items: [Item!]! }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let facts = SchemaFacts::of(&schema);
        let query = schema.types.get("Query").unwrap();

        let item = query.fields.iter().find(|f| f.name == "item").unwrap();
        let RootPlan::Get { members, .. } = classify(item, RootKind::Query, &schema, &graph, &facts)
        else {
            panic!("item should be a lookup")
        };
        assert_eq!(members.len(), 2, "both union members should be reachable");

        let items = query.fields.iter().find(|f| f.name == "items").unwrap();
        let RootPlan::List { members, .. } =
            classify(items, RootKind::Query, &schema, &graph, &facts)
        else {
            panic!("items should be a list")
        };
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn an_interface_return_type_finds_its_implementors() {
        let schema = parse_sdl(
            "interface Node { id: ID! } type A implements Node { id: ID! } \
             type B implements Node { id: ID! } type Query { node(id: ID!): Node }",
        )
        .unwrap();
        let graph = to_entity_graph(&schema);
        let facts = SchemaFacts::of(&schema);
        let query = schema.types.get("Query").unwrap();
        let node = query.fields.iter().find(|f| f.name == "node").unwrap();
        let RootPlan::Get { members, .. } = classify(node, RootKind::Query, &schema, &graph, &facts)
        else {
            panic!("node should be a lookup")
        };
        assert_eq!(members.len(), 2);
    }
}
