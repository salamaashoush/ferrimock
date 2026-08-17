//! GraphQL introspection type definitions

use std::fmt;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Full introspection query response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionResponse {
    pub data: IntrospectionData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionData {
    #[serde(rename = "__schema")]
    pub schema: SchemaIntrospection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaIntrospection {
    pub query_type: Option<TypeNameRef>,
    pub mutation_type: Option<TypeNameRef>,
    pub subscription_type: Option<TypeNameRef>,
    pub types: Vec<FullType>,
    #[serde(default)]
    pub directives: Vec<DirectiveIntrospection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeNameRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullType {
    pub kind: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<FieldIntrospection>>,
    #[serde(default)]
    pub input_fields: Option<Vec<InputValueIntrospection>>,
    #[serde(default)]
    pub interfaces: Option<Vec<TypeRefIntrospection>>,
    #[serde(default)]
    pub enum_values: Option<Vec<EnumValueIntrospection>>,
    #[serde(default)]
    pub possible_types: Option<Vec<TypeRefIntrospection>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldIntrospection {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub args: Vec<InputValueIntrospection>,
    #[serde(rename = "type")]
    pub field_type: TypeRefIntrospection,
    #[serde(default)]
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputValueIntrospection {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub value_type: TypeRefIntrospection,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeRefIntrospection {
    pub kind: String,
    pub name: Option<String>,
    pub of_type: Option<Box<TypeRefIntrospection>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueIntrospection {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectiveIntrospection {
    pub name: String,
    pub description: Option<String>,
    pub locations: Vec<String>,
    #[serde(default)]
    pub args: Vec<InputValueIntrospection>,
}

/// Parsed and structured GraphQL schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSchema {
    pub query_type: Option<String>,
    pub mutation_type: Option<String>,
    pub subscription_type: Option<String>,
    pub types: FxHashMap<String, TypeDefinition>,
    pub directives: Vec<DirectiveDefinition>,
}

/// Type definition after parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDefinition {
    pub kind: TypeKind,
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<FieldDefinition>,
    pub input_fields: Vec<InputValueDefinition>,
    pub interfaces: Vec<TypeRef>,
    pub enum_values: Vec<EnumValueDefinition>,
    pub possible_types: Vec<TypeRef>,
}

/// GraphQL type kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeKind {
    Scalar,
    Object,
    Interface,
    Union,
    Enum,
    InputObject,
    List,
    NonNull,
}

impl TypeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "SCALAR" => Some(TypeKind::Scalar),
            "OBJECT" => Some(TypeKind::Object),
            "INTERFACE" => Some(TypeKind::Interface),
            "UNION" => Some(TypeKind::Union),
            "ENUM" => Some(TypeKind::Enum),
            "INPUT_OBJECT" => Some(TypeKind::InputObject),
            "LIST" => Some(TypeKind::List),
            "NON_NULL" => Some(TypeKind::NonNull),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TypeKind::Scalar => "SCALAR",
            TypeKind::Object => "OBJECT",
            TypeKind::Interface => "INTERFACE",
            TypeKind::Union => "UNION",
            TypeKind::Enum => "ENUM",
            TypeKind::InputObject => "INPUT_OBJECT",
            TypeKind::List => "LIST",
            TypeKind::NonNull => "NON_NULL",
        }
    }
}

/// Field definition after parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub name: String,
    pub description: Option<String>,
    pub args: Vec<InputValueDefinition>,
    pub field_type: TypeRef,
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

/// Input value (argument or input field) definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputValueDefinition {
    pub name: String,
    pub description: Option<String>,
    pub value_type: TypeRef,
    pub default_value: Option<String>,
}

/// Enum value definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumValueDefinition {
    pub name: String,
    pub description: Option<String>,
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

/// Directive definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectiveDefinition {
    pub name: String,
    pub description: Option<String>,
    pub locations: Vec<String>,
    pub args: Vec<InputValueDefinition>,
}

/// Type reference, keeping every NON_NULL and LIST wrapper.
///
/// The wrappers nest arbitrarily (`[[String!]!]`), so they are represented as
/// they are written rather than flattened into booleans: a flattened form
/// cannot tell `[String!]` from `[String]!`, and loses inner lists entirely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeRef {
    Named(String),
    NonNull(Box<TypeRef>),
    List(Box<TypeRef>),
}

impl TypeRef {
    /// Create a new type reference from introspection data
    pub fn from_introspection(intro: &TypeRefIntrospection) -> Self {
        match intro.kind.as_str() {
            "NON_NULL" => intro.of_type.as_ref().map_or_else(
                || Self::Named(String::new()),
                |of_type| Self::NonNull(Box::new(Self::from_introspection(of_type))),
            ),
            "LIST" => intro.of_type.as_ref().map_or_else(
                || Self::List(Box::new(Self::Named(String::new()))),
                |of_type| Self::List(Box::new(Self::from_introspection(of_type))),
            ),
            _ => Self::Named(intro.name.clone().unwrap_or_default()),
        }
    }

    /// Build a named type reference
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }

    /// Wrap in NON_NULL
    #[must_use]
    pub fn non_null(self) -> Self {
        Self::NonNull(Box::new(self))
    }

    /// Wrap in LIST
    #[must_use]
    pub fn list(self) -> Self {
        Self::List(Box::new(self))
    }

    /// The innermost named type, with every wrapper stripped
    pub fn name(&self) -> &str {
        match self {
            Self::Named(name) => name,
            Self::NonNull(inner) | Self::List(inner) => inner.name(),
        }
    }

    /// Whether the outermost wrapper is NON_NULL
    pub fn is_non_null(&self) -> bool {
        matches!(self, Self::NonNull(_))
    }

    /// Whether this is a list once its own nullability is stripped
    pub fn is_list(&self) -> bool {
        match self {
            Self::List(_) => true,
            Self::NonNull(inner) => matches!(**inner, Self::List(_)),
            Self::Named(_) => false,
        }
    }

    /// For a list, whether its elements are non-null
    pub fn inner_non_null(&self) -> bool {
        match self {
            Self::List(inner) => inner.is_non_null(),
            Self::NonNull(inner) => inner.inner_non_null(),
            Self::Named(_) => false,
        }
    }

    /// The element type of a list, with the list wrapper removed
    pub fn list_item(&self) -> Option<&Self> {
        match self {
            Self::List(inner) => Some(inner),
            Self::NonNull(inner) => inner.list_item(),
            Self::Named(_) => None,
        }
    }

    /// Get the unwrapped type information
    pub fn unwrap(&self) -> UnwrappedType {
        UnwrappedType {
            name: self.name().to_string(),
            is_non_null: self.is_non_null(),
            is_list: self.is_list(),
            inner_non_null: self.inner_non_null(),
        }
    }
}

impl fmt::Display for TypeRef {
    /// Format as SDL type notation (e.g., `String!`, `[User!]!`, `[[Int]]`)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "{name}"),
            Self::NonNull(inner) => write!(f, "{inner}!"),
            Self::List(inner) => write!(f, "[{inner}]"),
        }
    }
}

/// Unwrapped type information
#[derive(Debug, Clone)]
pub struct UnwrappedType {
    pub name: String,
    pub is_non_null: bool,
    pub is_list: bool,
    pub inner_non_null: bool,
}

/// GraphQL operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    Query,
    Mutation,
    Subscription,
}

impl OperationType {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationType::Query => "query",
            OperationType::Mutation => "mutation",
            OperationType::Subscription => "subscription",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod type_ref_tests {
    use super::*;

    fn named(name: &str) -> TypeRefIntrospection {
        TypeRefIntrospection {
            kind: "SCALAR".to_string(),
            name: Some(name.to_string()),
            of_type: None,
        }
    }

    fn wrap(kind: &str, inner: TypeRefIntrospection) -> TypeRefIntrospection {
        TypeRefIntrospection {
            kind: kind.to_string(),
            name: None,
            of_type: Some(Box::new(inner)),
        }
    }

    #[test]
    fn list_of_non_null_is_not_itself_non_null() {
        let t = TypeRef::from_introspection(&wrap("LIST", wrap("NON_NULL", named("String"))));
        assert_eq!(t.to_string(), "[String!]");
        assert!(!t.is_non_null());
        assert!(t.is_list());
        assert!(t.inner_non_null());
    }

    #[test]
    fn non_null_list_is_distinct_from_list_of_non_null() {
        let t = TypeRef::from_introspection(&wrap("NON_NULL", wrap("LIST", named("String"))));
        assert_eq!(t.to_string(), "[String]!");
        assert!(t.is_non_null());
        assert!(t.is_list());
        assert!(!t.inner_non_null());
    }

    #[test]
    fn nested_lists_survive() {
        let t = TypeRef::from_introspection(&wrap("LIST", wrap("LIST", named("String"))));
        assert_eq!(t.to_string(), "[[String]]");
        assert_eq!(t.name(), "String");
    }

    #[test]
    fn deeply_wrapped_round_trips() {
        let t = TypeRef::from_introspection(&wrap(
            "NON_NULL",
            wrap(
                "LIST",
                wrap("NON_NULL", wrap("LIST", wrap("NON_NULL", named("Int")))),
            ),
        ));
        assert_eq!(t.to_string(), "[[Int!]!]!");
    }
}
