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
#[derive(Debug, Clone)]
pub struct ParsedSchema {
    pub query_type: Option<String>,
    pub mutation_type: Option<String>,
    pub subscription_type: Option<String>,
    pub types: FxHashMap<String, TypeDefinition>,
    pub directives: Vec<DirectiveDefinition>,
}

/// Type definition after parsing
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub name: String,
    pub description: Option<String>,
    pub args: Vec<InputValueDefinition>,
    pub field_type: TypeRef,
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

/// Input value (argument or input field) definition
#[derive(Debug, Clone)]
pub struct InputValueDefinition {
    pub name: String,
    pub description: Option<String>,
    pub value_type: TypeRef,
    pub default_value: Option<String>,
}

/// Enum value definition
#[derive(Debug, Clone)]
pub struct EnumValueDefinition {
    pub name: String,
    pub description: Option<String>,
    pub is_deprecated: bool,
    pub deprecation_reason: Option<String>,
}

/// Directive definition
#[derive(Debug, Clone)]
pub struct DirectiveDefinition {
    pub name: String,
    pub description: Option<String>,
    pub locations: Vec<String>,
    pub args: Vec<InputValueDefinition>,
}

/// Type reference with wrappers (NON_NULL, LIST)
#[derive(Debug, Clone)]
pub struct TypeRef {
    /// The innermost type name
    pub name: String,
    /// Whether this is wrapped in NON_NULL
    pub is_non_null: bool,
    /// Whether this is wrapped in LIST (can be nested)
    pub is_list: bool,
    /// For nested lists: is the inner type non-null?
    pub inner_non_null: bool,
}

impl TypeRef {
    /// Create a new type reference from introspection data
    pub fn from_introspection(intro: &TypeRefIntrospection) -> Self {
        Self::unwrap_type(intro)
    }

    /// Recursively unwrap NON_NULL and LIST wrappers
    fn unwrap_type(intro: &TypeRefIntrospection) -> Self {
        match intro.kind.as_str() {
            "NON_NULL" => {
                if let Some(of_type) = &intro.of_type {
                    let mut inner = Self::unwrap_type(of_type);
                    inner.is_non_null = true;
                    inner
                } else {
                    Self {
                        name: String::new(),
                        is_non_null: true,
                        is_list: false,
                        inner_non_null: false,
                    }
                }
            }
            "LIST" => {
                if let Some(of_type) = &intro.of_type {
                    let mut inner = Self::unwrap_type(of_type);
                    inner.is_list = true;
                    // Check if the list item is non-null
                    if of_type.kind == "NON_NULL" {
                        inner.inner_non_null = true;
                    }
                    inner
                } else {
                    Self {
                        name: String::new(),
                        is_non_null: false,
                        is_list: true,
                        inner_non_null: false,
                    }
                }
            }
            _ => {
                // Named type (SCALAR, OBJECT, etc.)
                Self {
                    name: intro.name.clone().unwrap_or_default(),
                    is_non_null: false,
                    is_list: false,
                    inner_non_null: false,
                }
            }
        }
    }

    /// Get the unwrapped type information
    pub fn unwrap(&self) -> UnwrappedType {
        UnwrappedType {
            name: self.name.clone(),
            is_non_null: self.is_non_null,
            is_list: self.is_list,
            inner_non_null: self.inner_non_null,
        }
    }
}

impl fmt::Display for TypeRef {
    /// Format as SDL type notation (e.g., `String!`, `[User!]!`, `[Int]`)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_list {
            if self.inner_non_null {
                write!(f, "[{}!]", self.name)?;
            } else {
                write!(f, "[{}]", self.name)?;
            }
        } else {
            write!(f, "{}", self.name)?;
        }

        if self.is_non_null {
            write!(f, "!")?;
        }

        Ok(())
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
