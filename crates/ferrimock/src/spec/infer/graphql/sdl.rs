//! Reading SDL into the same [`ParsedSchema`] introspection produces.
//!
//! The engine could only ever consume a live endpoint's introspection JSON;
//! a `.graphql` file — the thing people actually have in a repository — had no
//! way in. Both inputs converge here so everything downstream sees one shape,
//! and `generate_sdl` round-trips.

use async_graphql_parser::types as ast;
use rustc_hash::FxHashMap;

use crate::graphql::introspection::{
    DirectiveDefinition, EnumValueDefinition, FieldDefinition, InputValueDefinition, ParsedSchema,
    TypeDefinition, TypeKind, TypeRef,
};

/// Parse a GraphQL SDL document.
///
/// A grammar error points at the byte where the parser gave up, which for a
/// malformed description is nowhere near the mistake. When the source has a
/// defect this crate can name, that name is the error instead.
pub fn parse_sdl(source: &str) -> crate::Result<ParsedSchema> {
    match async_graphql_parser::parse_schema(source) {
        Ok(document) => Ok(from_document(&document)),
        Err(error) => {
            let defects = super::defects::find_defects(source);
            if defects.is_empty() {
                return Err(crate::mp_err!("Invalid GraphQL SDL: {error}"));
            }
            let listed = defects
                .iter()
                .take(5)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n  ");
            let more = defects.len().saturating_sub(5);
            let suffix = if more > 0 {
                format!("\n  ...and {more} more")
            } else {
                String::new()
            };
            Err(crate::mp_err!(
                "Invalid GraphQL SDL: {} malformed description(s)\n  {listed}{suffix}",
                defects.len()
            ))
        }
    }
}

/// Parse SDL, repairing malformations this crate knows how to repair.
///
/// Returns what was repaired so a caller can report it. Never silent: a tool
/// that rewrites its input without saying so cannot be trusted with the next
/// file.
pub fn parse_sdl_lenient(source: &str) -> crate::Result<(ParsedSchema, Vec<super::defects::SdlDefect>)> {
    if let Ok(document) = async_graphql_parser::parse_schema(source) {
        return Ok((from_document(&document), Vec::new()));
    }
    let (repaired, defects) = super::defects::repair(source);
    let document = async_graphql_parser::parse_schema(&repaired)
        .map_err(|e| crate::mp_err!("Invalid GraphQL SDL, beyond repair: {e}"))?;
    Ok((from_document(&document), defects))
}

fn from_document(document: &ast::ServiceDocument) -> ParsedSchema {
    let mut types: FxHashMap<String, TypeDefinition> = FxHashMap::default();
    let mut directives = Vec::new();
    let mut roots = (None, None, None);

    for definition in &document.definitions {
        match definition {
            ast::TypeSystemDefinition::Schema(schema) => {
                let node = &schema.node;
                if let Some(name) = &node.query {
                    roots.0 = Some(name.node.to_string());
                }
                if let Some(name) = &node.mutation {
                    roots.1 = Some(name.node.to_string());
                }
                if let Some(name) = &node.subscription {
                    roots.2 = Some(name.node.to_string());
                }
            }
            ast::TypeSystemDefinition::Type(type_def) => {
                let parsed = type_definition(&type_def.node);
                match types.get_mut(&parsed.name) {
                    // `extend type` merges into what is already there rather
                    // than replacing it, which is the whole point of extension.
                    Some(existing) if type_def.node.extend => merge_into(existing, parsed),
                    _ => {
                        types.insert(parsed.name.clone(), parsed);
                    }
                }
            }
            ast::TypeSystemDefinition::Directive(directive) => {
                let node = &directive.node;
                directives.push(DirectiveDefinition {
                    name: node.name.node.to_string(),
                    description: node.description.as_ref().map(|d| d.node.clone()),
                    locations: node
                        .locations
                        .iter()
                        .map(|l| directive_location(l.node))
                        .collect(),
                    args: node
                        .arguments
                        .iter()
                        .map(|a| input_value(&a.node))
                        .collect(),
                });
            }
        }
    }

    // A schema with no explicit `schema { }` block uses the conventional root
    // names, which is how most hand-written SDL is spelled.
    let (query, mutation, subscription) = roots;
    let query = query.or_else(|| types.contains_key("Query").then(|| "Query".to_string()));
    let mutation =
        mutation.or_else(|| types.contains_key("Mutation").then(|| "Mutation".to_string()));
    let subscription = subscription.or_else(|| {
        types
            .contains_key("Subscription")
            .then(|| "Subscription".to_string())
    });

    ParsedSchema {
        query_type: query,
        mutation_type: mutation,
        subscription_type: subscription,
        types,
        directives,
    }
}

fn merge_into(existing: &mut TypeDefinition, extension: TypeDefinition) {
    existing.fields.extend(extension.fields);
    existing.input_fields.extend(extension.input_fields);
    existing.interfaces.extend(extension.interfaces);
    existing.enum_values.extend(extension.enum_values);
    existing.possible_types.extend(extension.possible_types);
}

fn type_definition(node: &ast::TypeDefinition) -> TypeDefinition {
    let name = node.name.node.to_string();
    let description = node.description.as_ref().map(|d| d.node.clone());

    let mut definition = TypeDefinition {
        kind: TypeKind::Scalar,
        name,
        description,
        fields: Vec::new(),
        input_fields: Vec::new(),
        interfaces: Vec::new(),
        enum_values: Vec::new(),
        possible_types: Vec::new(),
    };

    match &node.kind {
        ast::TypeKind::Scalar => definition.kind = TypeKind::Scalar,
        ast::TypeKind::Object(object) => {
            definition.kind = TypeKind::Object;
            definition.fields = object.fields.iter().map(|f| field(&f.node)).collect();
            definition.interfaces = object
                .implements
                .iter()
                .map(|i| TypeRef::named(i.node.to_string()))
                .collect();
        }
        ast::TypeKind::Interface(interface) => {
            definition.kind = TypeKind::Interface;
            definition.fields = interface.fields.iter().map(|f| field(&f.node)).collect();
            definition.interfaces = interface
                .implements
                .iter()
                .map(|i| TypeRef::named(i.node.to_string()))
                .collect();
        }
        ast::TypeKind::Union(union) => {
            definition.kind = TypeKind::Union;
            definition.possible_types = union
                .members
                .iter()
                .map(|m| TypeRef::named(m.node.to_string()))
                .collect();
        }
        ast::TypeKind::Enum(enumeration) => {
            definition.kind = TypeKind::Enum;
            definition.enum_values = enumeration
                .values
                .iter()
                .map(|v| {
                    let value = &v.node;
                    EnumValueDefinition {
                        name: value.value.node.to_string(),
                        description: value.description.as_ref().map(|d| d.node.clone()),
                        is_deprecated: is_deprecated(&value.directives),
                        deprecation_reason: deprecation_reason(&value.directives),
                    }
                })
                .collect();
        }
        ast::TypeKind::InputObject(input) => {
            definition.kind = TypeKind::InputObject;
            definition.input_fields = input
                .fields
                .iter()
                .map(|f| input_value(&f.node))
                .collect();
        }
    }

    definition
}

fn field(node: &ast::FieldDefinition) -> FieldDefinition {
    FieldDefinition {
        name: node.name.node.to_string(),
        description: node.description.as_ref().map(|d| d.node.clone()),
        args: node
            .arguments
            .iter()
            .map(|a| input_value(&a.node))
            .collect(),
        field_type: type_ref(&node.ty.node),
        is_deprecated: is_deprecated(&node.directives),
        deprecation_reason: deprecation_reason(&node.directives),
    }
}

fn input_value(node: &ast::InputValueDefinition) -> InputValueDefinition {
    InputValueDefinition {
        name: node.name.node.to_string(),
        description: node.description.as_ref().map(|d| d.node.clone()),
        value_type: type_ref(&node.ty.node),
        default_value: node.default_value.as_ref().map(|v| v.node.to_string()),
    }
}

fn type_ref(ty: &ast::Type) -> TypeRef {
    let base = match &ty.base {
        ast::BaseType::Named(name) => TypeRef::named(name.to_string()),
        ast::BaseType::List(inner) => TypeRef::list(type_ref(inner)),
    };
    if ty.nullable { base } else { base.non_null() }
}

/// The spelling a directive location has in SDL.
///
/// Debug-formatting the parser's enum drops the underscores, so a round trip
/// through `generate_sdl` would emit `FIELDDEFINITION` and stop parsing.
fn directive_location(location: ast::DirectiveLocation) -> String {
    use ast::DirectiveLocation as L;
    match location {
        L::Query => "QUERY",
        L::Mutation => "MUTATION",
        L::Subscription => "SUBSCRIPTION",
        L::Field => "FIELD",
        L::FragmentDefinition => "FRAGMENT_DEFINITION",
        L::FragmentSpread => "FRAGMENT_SPREAD",
        L::InlineFragment => "INLINE_FRAGMENT",
        L::Schema => "SCHEMA",
        L::Scalar => "SCALAR",
        L::Object => "OBJECT",
        L::FieldDefinition => "FIELD_DEFINITION",
        L::ArgumentDefinition => "ARGUMENT_DEFINITION",
        L::Interface => "INTERFACE",
        L::Union => "UNION",
        L::Enum => "ENUM",
        L::EnumValue => "ENUM_VALUE",
        L::InputObject => "INPUT_OBJECT",
        L::InputFieldDefinition => "INPUT_FIELD_DEFINITION",
        L::VariableDefinition => "VARIABLE_DEFINITION",
    }
    .to_string()
}

fn is_deprecated(directives: &[async_graphql_parser::Positioned<ast::ConstDirective>]) -> bool {
    directives
        .iter()
        .any(|d| d.node.name.node.as_str() == "deprecated")
}

fn deprecation_reason(
    directives: &[async_graphql_parser::Positioned<ast::ConstDirective>],
) -> Option<String> {
    directives
        .iter()
        .find(|d| d.node.name.node.as_str() == "deprecated")?
        .node
        .arguments
        .iter()
        .find(|(name, _)| name.node.as_str() == "reason")
        .map(|(_, value)| match &value.node {
            async_graphql::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::get_unwrap, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const BLOG: &str = r#"
        "A person who writes."
        type User implements Node {
          id: ID!
          name: String!
          email: String
          posts: [Post!]!
        }

        interface Node { id: ID! }

        type Post {
          id: ID!
          title: String!
          tags: [[String]]
          status: Status!
          author: User!
          legacy: String @deprecated(reason: "use title")
        }

        enum Status { DRAFT PUBLISHED }

        union Content = Post | User

        input PostFilter { status: Status, limit: Int = 10 }

        type Query {
          user(id: ID!): User
          posts(first: Int, after: String, filter: PostFilter): [Post!]!
        }

        type Mutation { createPost(title: String!): Post! }
    "#;

    #[test]
    fn objects_fields_and_wrappers_survive() {
        let schema = parse_sdl(BLOG).unwrap();
        let user = schema.types.get("User").unwrap();
        assert_eq!(user.kind, TypeKind::Object);
        assert_eq!(user.description.as_deref(), Some("A person who writes."));

        let posts = user.fields.iter().find(|f| f.name == "posts").unwrap();
        assert_eq!(posts.field_type.to_string(), "[Post!]!");

        let email = user.fields.iter().find(|f| f.name == "email").unwrap();
        assert_eq!(email.field_type.to_string(), "String");
    }

    #[test]
    fn nested_list_wrappers_survive() {
        let schema = parse_sdl(BLOG).unwrap();
        let post = schema.types.get("Post").unwrap();
        let tags = post.fields.iter().find(|f| f.name == "tags").unwrap();
        assert_eq!(tags.field_type.to_string(), "[[String]]");
    }

    #[test]
    fn roots_default_to_the_conventional_names() {
        let schema = parse_sdl(BLOG).unwrap();
        assert_eq!(schema.query_type.as_deref(), Some("Query"));
        assert_eq!(schema.mutation_type.as_deref(), Some("Mutation"));
        assert_eq!(schema.subscription_type, None);
    }

    #[test]
    fn an_explicit_schema_block_wins() {
        let schema = parse_sdl(
            "schema { query: Root } type Root { a: String } type Query { b: String }",
        )
        .unwrap();
        assert_eq!(schema.query_type.as_deref(), Some("Root"));
    }

    #[test]
    fn interfaces_unions_enums_and_inputs_are_captured() {
        let schema = parse_sdl(BLOG).unwrap();
        assert_eq!(schema.types.get("Node").unwrap().kind, TypeKind::Interface);
        assert_eq!(
            schema.types.get("User").unwrap().interfaces[0].name(),
            "Node"
        );

        let content = schema.types.get("Content").unwrap();
        assert_eq!(content.kind, TypeKind::Union);
        let members: Vec<_> = content.possible_types.iter().map(TypeRef::name).collect();
        assert_eq!(members, ["Post", "User"]);

        let status = schema.types.get("Status").unwrap();
        assert_eq!(status.kind, TypeKind::Enum);
        let values: Vec<_> = status.enum_values.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(values, ["DRAFT", "PUBLISHED"]);

        let filter = schema.types.get("PostFilter").unwrap();
        assert_eq!(filter.kind, TypeKind::InputObject);
        let limit = filter
            .input_fields
            .iter()
            .find(|f| f.name == "limit")
            .unwrap();
        assert_eq!(limit.default_value.as_deref(), Some("10"));
    }

    #[test]
    fn field_arguments_survive() {
        let schema = parse_sdl(BLOG).unwrap();
        let query = schema.types.get("Query").unwrap();
        let user = query.fields.iter().find(|f| f.name == "user").unwrap();
        assert_eq!(user.args.len(), 1);
        assert_eq!(user.args[0].name, "id");
        assert_eq!(user.args[0].value_type.to_string(), "ID!");
    }

    #[test]
    fn deprecation_survives() {
        let schema = parse_sdl(BLOG).unwrap();
        let post = schema.types.get("Post").unwrap();
        let legacy = post.fields.iter().find(|f| f.name == "legacy").unwrap();
        assert!(legacy.is_deprecated);
        assert_eq!(legacy.deprecation_reason.as_deref(), Some("use title"));
    }

    #[test]
    fn an_extension_merges_rather_than_replaces() {
        let schema = parse_sdl(
            "type User { id: ID! } extend type User { nickname: String }",
        )
        .unwrap();
        let user = schema.types.get("User").unwrap();
        let names: Vec<_> = user.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["id", "nickname"]);
    }

    #[test]
    fn sdl_round_trips_through_the_generator() {
        let schema = parse_sdl(BLOG).unwrap();
        let regenerated = crate::graphql::generate_sdl(&schema);
        let reparsed = parse_sdl(&regenerated).unwrap();

        let original_post = schema.types.get("Post").unwrap();
        let round_tripped = reparsed.types.get("Post").unwrap();
        for field in &original_post.fields {
            let same = round_tripped
                .fields
                .iter()
                .find(|f| f.name == field.name)
                .unwrap_or_else(|| panic!("`{}` lost in the round trip", field.name));
            assert_eq!(
                same.field_type.to_string(),
                field.field_type.to_string(),
                "`{}` changed type in the round trip",
                field.name
            );
        }
    }

    /// Directive locations are spelled in SCREAMING_SNAKE_CASE. Debug-format
    /// drops the underscores, which survived a parse but broke the very next
    /// `generate_sdl` — only a round trip catches it.
    #[test]
    fn directive_locations_survive_a_round_trip() {
        let schema = parse_sdl(
            "directive @auth on FIELD_DEFINITION | OBJECT\n\
             directive @tag(name: String!) on ENUM_VALUE | INPUT_FIELD_DEFINITION\n\
             type Query { a: String }",
        )
        .unwrap();

        let auth = schema.directives.iter().find(|d| d.name == "auth").unwrap();
        assert_eq!(auth.locations, ["FIELD_DEFINITION", "OBJECT"]);

        let regenerated = crate::graphql::generate_sdl(&schema);
        assert!(
            regenerated.contains("FIELD_DEFINITION"),
            "underscores must survive: {regenerated}"
        );
        assert!(
            parse_sdl(&regenerated).is_ok(),
            "the emitted SDL must parse again:\n{regenerated}"
        );
    }

    /// The property that matters for a generator: whatever goes in comes back
    /// out, and what comes out parses.
    #[test]
    fn a_whole_schema_round_trips_through_the_generator() {
        let schema = parse_sdl(BLOG).unwrap();
        let once = crate::graphql::generate_sdl(&schema);
        let reparsed = parse_sdl(&once).unwrap();
        let twice = crate::graphql::generate_sdl(&reparsed);
        assert_eq!(
            once, twice,
            "a second pass must not keep changing the output"
        );
        assert_eq!(schema.types.len(), reparsed.types.len());
    }

    #[test]
    fn invalid_sdl_is_an_error_rather_than_an_empty_schema() {
        assert!(parse_sdl("type { }").is_err());
    }
}
