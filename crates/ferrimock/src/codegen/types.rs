//! Template codegen types
//!
//! These types are independent of the consolidator and define the interface
//! for template generation. The consolidator maps its analysis types to these.

use crate::type_detector::FieldType;
use rustc_hash::FxHashMap;
use serde_json::Value as JsonValue;
use std::sync::LazyLock;

/// Pagination pattern for template generation
#[derive(Debug, Clone)]
pub struct PaginationInfo {
    /// Total count field name (e.g., "total_count", "total")
    pub total_field: Option<String>,
    /// Offset field name (e.g., "offset", "skip")
    pub offset_field: Option<String>,
    /// Limit field name (e.g., "limit", "per_page")
    pub limit_field: Option<String>,
    /// Next marker/cursor field (e.g., "next_marker")
    pub next_field: Option<String>,
    /// Previous marker/cursor field (e.g., "prev_marker")
    pub prev_field: Option<String>,
    /// Has more field (e.g., "has_more")
    pub has_more_field: Option<String>,
    /// Sample total value for default
    pub sample_total: Option<i64>,
    /// Pagination type
    pub pagination_type: PaginationType,
    /// Static query parameters
    pub static_query_params: String,
    /// Where the recording's own pagination links pointed, without their query.
    pub link_base: Option<String>,
    /// The query parameter the client sends a cursor back in.
    pub cursor_param: Option<String>,
}

/// Type of pagination
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationType {
    Offset,
    Cursor,
    Page,
}

/// Response structure analysis for template generation
#[derive(Debug)]
pub struct ResponseStructure {
    /// Fields that vary across responses
    pub varying_fields: Vec<(String, FieldType)>,
    /// Fields that are constant
    pub constant_fields: Vec<(String, JsonValue)>,
    /// Response fields that repeated a value the request carried, keyed by the
    /// field's path in the response: `id`, `parent.id`, `entries[].parent.id`.
    ///
    /// What makes a merged mock answer about the thing that was asked for
    /// rather than about something it invented.
    pub echoed_fields: FxHashMap<String, EchoedField>,
    /// Whether response is JSON
    pub is_json: bool,
    /// Top-level type (object, array, etc.)
    pub top_level_type: String,
    /// Pagination information if detected
    pub pagination: Option<PaginationInfo>,
}

/// The part of the request a repeated value came from.
///
/// Variance lives wherever the client can put it, and the runtime already
/// offers all of these to a template. Which one a field repeated decides the
/// name the template reads it back from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EchoSource {
    /// A named capture in the URL pattern.
    Capture,
    /// A query parameter.
    Query,
    /// A field of the JSON request body, addressed by dotted path.
    Body,
    /// A request header.
    Header,
}

impl EchoSource {
    /// The template variable this source is read back from.
    fn variable(self) -> &'static str {
        match self {
            Self::Capture => "captures",
            Self::Query => "query",
            Self::Body => "body_json",
            Self::Header => "headers",
        }
    }

    /// Whether the name addresses a nested value rather than one map key.
    ///
    /// Only a body path is dotted. A query parameter or header named `a.b` is a
    /// single key, and splitting it would look up something that is not there.
    fn is_dotted_path(self) -> bool {
        matches!(self, Self::Body)
    }
}

/// A response field that repeats a value the request carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoedField {
    /// Where in the request the value came from.
    pub source: EchoSource,
    /// The capture, parameter, header or body path that carried it.
    pub name: String,
    /// Whether the recorded value was a JSON string, and so whether the
    /// substitution needs quoting.
    pub quoted: bool,
    /// What the field wrote before the repeated value, and after it.
    ///
    /// A field does not always repeat a request's value bare. Some APIs write a
    /// folder's typed id as `d_9848115997` and a file's as `f_27977065362` --
    /// the id the URL named, wearing a prefix that says which kind it is.
    /// Answering that with a value of its own contradicts the request just as
    /// plainly as getting the id wrong.
    pub prefix: String,
    pub suffix: String,
}

impl EchoedField {
    /// The template expression that puts the request's own value back.
    pub fn expression(&self) -> String {
        let mut lookup = self.source.variable().to_string();
        if self.source.is_dotted_path() {
            for component in self.name.split('.') {
                lookup.push_str(&access(component));
            }
        } else {
            lookup.push_str(&access(&self.name));
        }

        // An affix only ever sits inside a string: it is text the field wrote
        // around the value, and text in JSON is quoted.
        if self.quoted || !self.prefix.is_empty() || !self.suffix.is_empty() {
            format!("\"{}{{{{ {lookup} }}}}{}\"", self.prefix, self.suffix)
        } else {
            format!("{{{{ {lookup} }}}}")
        }
    }
}

/// Address one component of a lookup.
///
/// Dotted access only reads a bare identifier, and plenty of real parameter and
/// header names are not one -- `fileIDs[]`, `x-request-id`. Those are addressed
/// by subscript instead. The quotes are single because the expression ends up
/// inside a JSON string, where a double quote would arrive at the engine
/// escaped.
fn access(component: &str) -> String {
    let bare = !component.is_empty()
        && !component.starts_with(|c: char| c.is_ascii_digit())
        && component
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');

    if bare {
        format!(".{component}")
    } else {
        format!("['{component}']")
    }
}

/// What the response around a field knows while its expression is written.
#[derive(Clone, Copy)]
pub struct EmitContext<'a> {
    /// Fields that repeated what the request carried, keyed by response path.
    pub echoes: &'a FxHashMap<String, EchoedField>,
    /// GraphQL variables the group varied over.
    pub graphql: &'a GraphQLVariableInfo,
}

static NO_ECHOES: LazyLock<FxHashMap<String, EchoedField>> = LazyLock::new(FxHashMap::default);
static NO_GRAPHQL: LazyLock<GraphQLVariableInfo> = LazyLock::new(GraphQLVariableInfo::empty);

impl EmitContext<'_> {
    /// A context for a caller that has no request evidence to offer, such as the
    /// GraphQL schema generator, which builds responses from types alone.
    pub fn plain() -> Self {
        Self {
            echoes: &NO_ECHOES,
            graphql: &NO_GRAPHQL,
        }
    }

    /// The echo recorded for a field at `path`, if the recordings showed one.
    pub fn echo(&self, path: &str) -> Option<&EchoedField> {
        self.echoes.get(path)
    }
}

/// The path of `field` inside the response, given its parent's path.
pub fn child_path(parent: &str, field: &str) -> String {
    if parent.is_empty() {
        field.to_string()
    } else {
        format!("{parent}.{field}")
    }
}

/// The path shared by every element of the array at `parent`.
pub fn element_path(parent: &str) -> String {
    format!("{parent}[]")
}

/// GraphQL variable analysis for template generation
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
// Field names intentionally use `_variables` suffix to clearly distinguish between
// varying_variables and constant_variables, maintaining semantic clarity in GraphQL context
pub struct GraphQLVariableInfo {
    /// Variables that vary across mocks (e.g., `["id", "input.role"]`)
    pub varying_variables: Vec<String>,
    /// Variables that are constant with their values
    pub constant_variables: Vec<(String, JsonValue)>,
    /// Whether any variables exist
    pub has_variables: bool,
    /// Whether there are varying variables
    pub has_varying_variables: bool,
}

impl GraphQLVariableInfo {
    /// Create an empty analysis (for non-GraphQL)
    pub fn empty() -> Self {
        Self {
            varying_variables: vec![],
            constant_variables: vec![],
            has_variables: false,
            has_varying_variables: false,
        }
    }
}
