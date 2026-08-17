//! GraphQL SDL (Schema Definition Language) generator
//!
//! Generates valid GraphQL SDL from a parsed introspection schema.
//! Handles all type kinds, descriptions, directives, and default values.

use std::fmt::Write;

use super::types::*;

/// Escape a value for a single-line GraphQL string.
///
/// A description is a normal string, so an unescaped `"` inside one ends it
/// early and the rest of the line becomes syntax. Generators that interpolate
/// descriptions straight into the output produce SDL no conforming parser
/// accepts — `"The MIME type (e.g., "text/plain")."` is rejected by graphql-js
/// too, not just by ours.
fn escape_string_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str(r"\t"),
            '\r' => out.push_str(r"\r"),
            '\n' => out.push_str(r"\n"),
            '\u{8}' => out.push_str(r"\b"),
            '\u{c}' => out.push_str(r"\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Built-in scalar types that should be excluded from SDL output
const BUILTIN_SCALARS: &[&str] = &["ID", "String", "Int", "Float", "Boolean"];

/// Built-in directives that should be excluded from SDL output
const BUILTIN_DIRECTIVES: &[&str] = &["skip", "include", "deprecated", "specifiedBy"];

/// Generate a complete GraphQL SDL string from a parsed schema.
///
/// The output is deterministic: types are sorted by name for stable output.
/// Built-in scalars, introspection types (`__`-prefixed), and built-in
/// directives are excluded.
pub fn generate_sdl(schema: &ParsedSchema) -> String {
    let mut writer = SdlWriter::new(schema);
    writer.write_schema(schema);
    writer.into_string()
}

/// Efficient single-pass SDL writer with a pre-allocated string buffer.
struct SdlWriter {
    buf: String,
}

impl SdlWriter {
    fn new(schema: &ParsedSchema) -> Self {
        // Pre-allocate based on rough estimate (types * avg chars per type)
        let estimated_size = schema.types.len() * 200;
        Self {
            buf: String::with_capacity(estimated_size),
        }
    }

    fn into_string(self) -> String {
        self.buf
    }

    fn write_schema(&mut self, schema: &ParsedSchema) {
        // Write schema definition block if root types differ from defaults
        self.write_schema_definition(schema);

        // Collect and sort types for deterministic output
        let mut types: Vec<&TypeDefinition> = schema
            .types
            .values()
            .filter(|t| !t.name.starts_with("__"))
            .collect();
        types.sort_by(|a, b| a.name.cmp(&b.name));

        for type_def in types {
            match type_def.kind {
                TypeKind::Scalar => self.write_scalar(type_def),
                TypeKind::Object => self.write_object(type_def, schema),
                TypeKind::Interface => self.write_interface(type_def),
                TypeKind::Union => self.write_union(type_def),
                TypeKind::Enum => self.write_enum(type_def),
                TypeKind::InputObject => self.write_input_object(type_def),
                TypeKind::List | TypeKind::NonNull => {
                    // Wrapper types are not emitted as top-level definitions
                }
            }
        }

        // Write custom directives
        let mut directives: Vec<&DirectiveDefinition> = schema
            .directives
            .iter()
            .filter(|d| !BUILTIN_DIRECTIVES.contains(&d.name.as_str()))
            .collect();
        directives.sort_by(|a, b| a.name.cmp(&b.name));

        for directive in directives {
            self.write_directive(directive);
        }
    }

    fn write_schema_definition(&mut self, schema: &ParsedSchema) {
        let query_is_default = schema.query_type.as_deref() == Some("Query");
        let mutation_is_default = schema.mutation_type.as_deref() == Some("Mutation");
        let subscription_is_default = schema.subscription_type.as_deref() == Some("Subscription");

        // Only emit schema block if root types differ from GraphQL defaults,
        // or if there are no mutations/subscriptions (omit those lines)
        let needs_schema_block = !query_is_default
            || (schema.mutation_type.is_some() && !mutation_is_default)
            || (schema.subscription_type.is_some() && !subscription_is_default);

        if !needs_schema_block {
            return;
        }

        self.buf.push_str("schema {\n");
        if let Some(ref query) = schema.query_type {
            let _ = writeln!(self.buf, "  query: {query}");
        }
        if let Some(ref mutation) = schema.mutation_type {
            let _ = writeln!(self.buf, "  mutation: {mutation}");
        }
        if let Some(ref subscription) = schema.subscription_type {
            let _ = writeln!(self.buf, "  subscription: {subscription}");
        }
        self.buf.push_str("}\n\n");
    }

    fn write_description(&mut self, description: Option<&str>, indent: &str) {
        if let Some(desc) = description {
            let desc = desc.trim();
            if desc.is_empty() {
                return;
            }
            if desc.contains('\n') {
                // A block string ends at the first unescaped `"""`, so a
                // description containing one has to escape it.
                let _ = writeln!(self.buf, "{indent}\"\"\"");
                for line in desc.lines() {
                    let _ = writeln!(self.buf, "{indent}{}", line.replace(r#"""""#, r#"\""""#));
                }
                let _ = writeln!(self.buf, "{indent}\"\"\"");
            } else {
                let _ = writeln!(self.buf, "{indent}\"{}\"", escape_string_value(desc));
            }
        }
    }

    fn write_scalar(&mut self, type_def: &TypeDefinition) {
        if BUILTIN_SCALARS.contains(&type_def.name.as_str()) {
            return;
        }
        self.write_description(type_def.description.as_deref(), "");
        let _ = write!(self.buf, "scalar {}\n\n", type_def.name);
    }

    fn write_object(&mut self, type_def: &TypeDefinition, schema: &ParsedSchema) {
        self.write_description(type_def.description.as_deref(), "");

        let _ = write!(self.buf, "type {}", type_def.name);

        // Interfaces
        if !type_def.interfaces.is_empty() {
            self.buf.push_str(" implements ");
            let ifaces: Vec<String> = type_def
                .interfaces
                .iter()
                .map(|i| i.name().to_string())
                .collect();
            self.buf.push_str(&ifaces.join(" & "));
        }

        if type_def.fields.is_empty() {
            self.buf.push_str("\n\n");
            return;
        }

        self.buf.push_str(" {\n");

        // Sort fields by name for deterministic output, but keep root operation types unsorted
        let is_root_type = schema.query_type.as_deref() == Some(&type_def.name)
            || schema.mutation_type.as_deref() == Some(&type_def.name)
            || schema.subscription_type.as_deref() == Some(&type_def.name);

        let fields: Vec<&FieldDefinition> = if is_root_type {
            type_def.fields.iter().collect()
        } else {
            let mut sorted: Vec<&FieldDefinition> = type_def.fields.iter().collect();
            sorted.sort_by(|a, b| a.name.cmp(&b.name));
            sorted
        };

        for field in fields {
            self.write_field(field);
        }
        self.buf.push_str("}\n\n");
    }

    fn write_field(&mut self, field: &FieldDefinition) {
        self.write_description(field.description.as_deref(), "  ");

        let _ = write!(self.buf, "  {}", field.name);

        // Arguments
        if !field.args.is_empty() {
            self.write_arguments(&field.args);
        }

        let _ = write!(self.buf, ": {}", field.field_type);

        // Deprecation directive
        self.write_deprecation(field.is_deprecated, field.deprecation_reason.as_deref());

        self.buf.push('\n');
    }

    fn write_arguments(&mut self, args: &[InputValueDefinition]) {
        if args.len() == 1 {
            // Single argument on one line -- safe: length checked above
            let Some(arg) = args.first() else { return };
            self.buf.push('(');
            let _ = write!(self.buf, "{}: {}", arg.name, arg.value_type);
            if let Some(ref default) = arg.default_value {
                let _ = write!(self.buf, " = {default}");
            }
            self.buf.push(')');
        } else {
            // Multiple arguments on separate lines
            self.buf.push_str("(\n");
            for arg in args {
                self.write_description(arg.description.as_deref(), "    ");
                let _ = write!(self.buf, "    {}: {}", arg.name, arg.value_type);
                if let Some(ref default) = arg.default_value {
                    let _ = write!(self.buf, " = {default}");
                }
                self.buf.push('\n');
            }
            self.buf.push_str("  )");
        }
    }

    fn write_deprecation(&mut self, is_deprecated: bool, reason: Option<&str>) {
        if !is_deprecated {
            return;
        }

        match reason {
            Some(reason) if !reason.is_empty() => {
                let escaped = reason.replace('"', "\\\"");
                let _ = write!(self.buf, " @deprecated(reason: \"{escaped}\")");
            }
            _ => {
                self.buf.push_str(" @deprecated");
            }
        }
    }

    fn write_interface(&mut self, type_def: &TypeDefinition) {
        self.write_description(type_def.description.as_deref(), "");
        let _ = write!(self.buf, "interface {}", type_def.name);

        if type_def.fields.is_empty() {
            self.buf.push_str("\n\n");
            return;
        }

        self.buf.push_str(" {\n");
        let mut fields: Vec<&FieldDefinition> = type_def.fields.iter().collect();
        fields.sort_by(|a, b| a.name.cmp(&b.name));

        for field in fields {
            self.write_field(field);
        }
        self.buf.push_str("}\n\n");
    }

    fn write_union(&mut self, type_def: &TypeDefinition) {
        self.write_description(type_def.description.as_deref(), "");
        let _ = write!(self.buf, "union {}", type_def.name);

        if type_def.possible_types.is_empty() {
            self.buf.push_str("\n\n");
            return;
        }

        self.buf.push_str(" = ");
        let members: Vec<&str> = type_def.possible_types.iter().map(TypeRef::name).collect();
        self.buf.push_str(&members.join(" | "));
        self.buf.push_str("\n\n");
    }

    fn write_enum(&mut self, type_def: &TypeDefinition) {
        self.write_description(type_def.description.as_deref(), "");
        let _ = write!(self.buf, "enum {}", type_def.name);

        if type_def.enum_values.is_empty() {
            self.buf.push_str("\n\n");
            return;
        }

        self.buf.push_str(" {\n");
        for value in &type_def.enum_values {
            self.write_description(value.description.as_deref(), "  ");
            let _ = write!(self.buf, "  {}", value.name);
            self.write_deprecation(value.is_deprecated, value.deprecation_reason.as_deref());
            self.buf.push('\n');
        }
        self.buf.push_str("}\n\n");
    }

    fn write_input_object(&mut self, type_def: &TypeDefinition) {
        self.write_description(type_def.description.as_deref(), "");
        let _ = write!(self.buf, "input {}", type_def.name);

        if type_def.input_fields.is_empty() {
            self.buf.push_str("\n\n");
            return;
        }

        self.buf.push_str(" {\n");
        let mut fields: Vec<&InputValueDefinition> = type_def.input_fields.iter().collect();
        fields.sort_by(|a, b| a.name.cmp(&b.name));

        for field in fields {
            self.write_description(field.description.as_deref(), "  ");
            let _ = write!(self.buf, "  {}: {}", field.name, field.value_type);
            if let Some(ref default) = field.default_value {
                let _ = write!(self.buf, " = {default}");
            }
            self.buf.push('\n');
        }
        self.buf.push_str("}\n\n");
    }

    fn write_directive(&mut self, directive: &DirectiveDefinition) {
        self.write_description(directive.description.as_deref(), "");
        let _ = write!(self.buf, "directive @{}", directive.name);

        if !directive.args.is_empty() {
            self.write_arguments(&directive.args);
        }

        if !directive.locations.is_empty() {
            self.buf.push_str(" on ");
            self.buf.push_str(&directive.locations.join(" | "));
        }

        self.buf.push_str("\n\n");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::graphql::introspection::SchemaParser;

    /// Helper to build an IntrospectionResponse from JSON and generate SDL
    fn generate_sdl_from_json(json: serde_json::Value) -> String {
        let response: crate::graphql::introspection::IntrospectionResponse =
            serde_json::from_value(json).expect("Failed to deserialize introspection response");
        let schema = SchemaParser::parse(response).expect("Failed to parse schema");
        generate_sdl(&schema)
    }

    #[test]
    fn test_simple_object_type() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "hello",
                      "description": null,
                      "args": [],
                      "type": { "kind": "SCALAR", "name": "String", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "String",
                  "description": null,
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("type Query {\n"));
        assert!(sdl.contains("  hello: String\n"));
        // Built-in scalar String should not appear
        assert!(!sdl.contains("scalar String"));
    }

    #[test]
    fn test_field_with_arguments() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "user",
                      "description": null,
                      "args": [
                        {
                          "name": "id",
                          "description": null,
                          "type": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null }
                          },
                          "defaultValue": null
                        }
                      ],
                      "type": { "kind": "OBJECT", "name": "User", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "User",
                  "description": null,
                  "fields": [
                    {
                      "name": "id",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": { "kind": "SCALAR", "name": "String", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("user(id: ID!): User"));
        assert!(sdl.contains("type User {\n"));
        assert!(sdl.contains("  id: ID!\n"));
        assert!(sdl.contains("  name: String\n"));
    }

    #[test]
    fn test_enum_with_deprecated_values() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "status",
                      "description": null,
                      "args": [],
                      "type": { "kind": "ENUM", "name": "Status", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "ENUM",
                  "name": "Status",
                  "description": "Status of an item",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": [
                    { "name": "ACTIVE", "description": null, "isDeprecated": false, "deprecationReason": null },
                    { "name": "INACTIVE", "description": null, "isDeprecated": false, "deprecationReason": null },
                    { "name": "DELETED", "description": null, "isDeprecated": true, "deprecationReason": "Use INACTIVE instead" }
                  ],
                  "possibleTypes": null
                }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("\"Status of an item\""));
        assert!(sdl.contains("enum Status {\n"));
        assert!(sdl.contains("  ACTIVE\n"));
        assert!(sdl.contains("  INACTIVE\n"));
        assert!(sdl.contains("  DELETED @deprecated(reason: \"Use INACTIVE instead\")"));
    }

    #[test]
    fn test_union_type() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "search",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": { "kind": "UNION", "name": "SearchResult", "ofType": null }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "UNION",
                  "name": "SearchResult",
                  "description": null,
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": [
                    { "kind": "OBJECT", "name": "User", "ofType": null },
                    { "kind": "OBJECT", "name": "Post", "ofType": null }
                  ]
                },
                {
                  "kind": "OBJECT",
                  "name": "User",
                  "description": null,
                  "fields": [{ "name": "id", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "ID", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "Post",
                  "description": null,
                  "fields": [{ "name": "id", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "ID", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("union SearchResult = "));
        // Members might be in any order, just check both are present
        assert!(sdl.contains("User"));
        assert!(sdl.contains("Post"));
    }

    #[test]
    fn test_interface_type() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "node",
                      "description": null,
                      "args": [{ "name": "id", "description": null, "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null } }, "defaultValue": null }],
                      "type": { "kind": "INTERFACE", "name": "Node", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "INTERFACE",
                  "name": "Node",
                  "description": "An object with a globally unique ID",
                  "fields": [
                    {
                      "name": "id",
                      "description": "The globally unique ID",
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": [
                    { "kind": "OBJECT", "name": "User", "ofType": null }
                  ]
                },
                {
                  "kind": "OBJECT",
                  "name": "User",
                  "description": null,
                  "fields": [
                    { "name": "id", "description": null, "args": [], "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null } }, "isDeprecated": false, "deprecationReason": null },
                    { "name": "name", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }
                  ],
                  "inputFields": null,
                  "interfaces": [{ "kind": "INTERFACE", "name": "Node", "ofType": null }],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("interface Node {\n"));
        assert!(sdl.contains("  id: ID!\n"));
        assert!(sdl.contains("type User implements Node {\n"));
    }

    #[test]
    fn test_input_object_with_defaults() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": { "name": "Mutation" },
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [{ "name": "dummy", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "Mutation",
                  "description": null,
                  "fields": [
                    {
                      "name": "createUser",
                      "description": null,
                      "args": [
                        { "name": "input", "description": null, "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "INPUT_OBJECT", "name": "CreateUserInput", "ofType": null } }, "defaultValue": null }
                      ],
                      "type": { "kind": "OBJECT", "name": "User", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "INPUT_OBJECT",
                  "name": "CreateUserInput",
                  "description": "Input for creating a user",
                  "fields": null,
                  "inputFields": [
                    { "name": "name", "description": null, "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "String", "ofType": null } }, "defaultValue": null },
                    { "name": "role", "description": null, "type": { "kind": "ENUM", "name": "Role", "ofType": null }, "defaultValue": "\"USER\"" },
                    { "name": "email", "description": null, "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "defaultValue": null }
                  ],
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "User",
                  "description": null,
                  "fields": [{ "name": "id", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "ID", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "ENUM", "name": "Role", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": [{ "name": "USER", "description": null, "isDeprecated": false, "deprecationReason": null }, { "name": "ADMIN", "description": null, "isDeprecated": false, "deprecationReason": null }], "possibleTypes": null },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("input CreateUserInput {\n"));
        assert!(sdl.contains("  name: String!\n"));
        assert!(sdl.contains("  role: Role = \"USER\"\n"));
        assert!(sdl.contains("  email: String\n"));
    }

    #[test]
    fn test_non_null_and_list_combinations() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    {
                      "name": "tags",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "LIST",
                          "name": null,
                          "ofType": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": { "kind": "SCALAR", "name": "String", "ofType": null }
                          }
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "names",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "LIST",
                        "name": null,
                        "ofType": { "kind": "SCALAR", "name": "String", "ofType": null }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        // names: [String] and tags: [String!]! - but root type fields keep insertion order
        assert!(sdl.contains("[String!]!"));
        assert!(sdl.contains("[String]"));
    }

    #[test]
    fn test_description_strings() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": "The root query type",
                  "fields": [
                    {
                      "name": "user",
                      "description": "Fetch a user by ID",
                      "args": [{ "name": "id", "description": "The user ID", "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null } }, "defaultValue": null }],
                      "type": { "kind": "SCALAR", "name": "String", "ofType": null },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("\"The root query type\"\n"));
        assert!(sdl.contains("  \"Fetch a user by ID\"\n"));
    }

    #[test]
    fn test_multiline_description() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": "Line one\nLine two\nLine three",
                  "fields": [
                    { "name": "hello", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("\"\"\"\nLine one\nLine two\nLine three\n\"\"\""));
    }

    #[test]
    fn test_schema_definition_block() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "RootQuery" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "RootQuery",
                  "description": null,
                  "fields": [
                    { "name": "hello", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(sdl.contains("schema {\n  query: RootQuery\n}\n"));
    }

    #[test]
    fn test_default_root_types_no_schema_block() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [
                    { "name": "hello", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }
                  ],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(!sdl.contains("schema {"));
    }

    #[test]
    fn test_builtin_types_excluded() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [{ "name": "x", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "ID", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "Int", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "Float", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "Boolean", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null },
                { "kind": "SCALAR", "name": "DateTime", "description": "Custom scalar", "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(!sdl.contains("scalar ID"));
        assert!(!sdl.contains("scalar String"));
        assert!(!sdl.contains("scalar Int"));
        assert!(!sdl.contains("scalar Float"));
        assert!(!sdl.contains("scalar Boolean"));
        assert!(sdl.contains("scalar DateTime"));
    }

    #[test]
    fn test_introspection_types_excluded() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [{ "name": "x", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__Schema",
                  "description": null,
                  "fields": [{ "name": "types", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "__Type",
                  "description": null,
                  "fields": [{ "name": "name", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": []
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        assert!(!sdl.contains("__Schema"));
        assert!(!sdl.contains("__Type"));
    }

    #[test]
    fn test_custom_directive() {
        let json = serde_json::json!({
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": null,
              "subscriptionType": null,
              "types": [
                {
                  "kind": "OBJECT",
                  "name": "Query",
                  "description": null,
                  "fields": [{ "name": "x", "description": null, "args": [], "type": { "kind": "SCALAR", "name": "String", "ofType": null }, "isDeprecated": false, "deprecationReason": null }],
                  "inputFields": null,
                  "interfaces": [],
                  "enumValues": null,
                  "possibleTypes": null
                },
                { "kind": "SCALAR", "name": "String", "description": null, "fields": null, "inputFields": null, "interfaces": null, "enumValues": null, "possibleTypes": null }
              ],
              "directives": [
                { "name": "skip", "description": null, "locations": ["FIELD"], "args": [{ "name": "if", "description": null, "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "Boolean", "ofType": null } }, "defaultValue": null }] },
                { "name": "include", "description": null, "locations": ["FIELD"], "args": [] },
                { "name": "deprecated", "description": null, "locations": ["FIELD_DEFINITION"], "args": [] },
                { "name": "cacheControl", "description": "Cache control directive", "locations": ["FIELD_DEFINITION", "OBJECT"], "args": [{ "name": "maxAge", "description": null, "type": { "kind": "SCALAR", "name": "Int", "ofType": null }, "defaultValue": null }] }
              ]
            }
          }
        });

        let sdl = generate_sdl_from_json(json);
        // Built-in directives should be excluded
        assert!(!sdl.contains("directive @skip"));
        assert!(!sdl.contains("directive @include"));
        assert!(!sdl.contains("directive @deprecated"));
        // Custom directive should be included
        assert!(sdl.contains("directive @cacheControl"));
        assert!(sdl.contains("FIELD_DEFINITION | OBJECT"));
    }

    #[test]
    fn test_type_ref_display() {
        assert_eq!(TypeRef::named("String").to_string(), "String");
        assert_eq!(TypeRef::named("String").non_null().to_string(), "String!");
        assert_eq!(TypeRef::named("String").list().to_string(), "[String]");
        assert_eq!(
            TypeRef::named("String")
                .non_null()
                .list()
                .non_null()
                .to_string(),
            "[String!]!"
        );
        assert_eq!(
            TypeRef::named("User").non_null().list().to_string(),
            "[User!]"
        );
        assert_eq!(
            TypeRef::named("Int").list().list().to_string(),
            "[[Int]]",
            "inner list wrappers must survive"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::get_unwrap)]
mod description_escaping_tests {
    use super::*;
    use crate::graphql::introspection::{FieldDefinition, ParsedSchema, TypeDefinition, TypeKind};
    use rustc_hash::FxHashMap;

    fn schema_with_description(description: &str) -> ParsedSchema {
        let mut types = FxHashMap::default();
        types.insert(
            "Thing".to_string(),
            TypeDefinition {
                kind: TypeKind::Object,
                name: "Thing".to_string(),
                description: None,
                fields: vec![FieldDefinition {
                    name: "mediaType".to_string(),
                    description: Some(description.to_string()),
                    args: Vec::new(),
                    field_type: TypeRef::named("String"),
                    is_deprecated: false,
                    deprecation_reason: None,
                }],
                input_fields: Vec::new(),
                interfaces: Vec::new(),
                enum_values: Vec::new(),
                possible_types: Vec::new(),
            },
        );
        types.insert(
            "Query".to_string(),
            TypeDefinition {
                kind: TypeKind::Object,
                name: "Query".to_string(),
                description: None,
                fields: vec![FieldDefinition {
                    name: "thing".to_string(),
                    description: None,
                    args: Vec::new(),
                    field_type: TypeRef::named("Thing"),
                    is_deprecated: false,
                    deprecation_reason: None,
                }],
                input_fields: Vec::new(),
                interfaces: Vec::new(),
                enum_values: Vec::new(),
                possible_types: Vec::new(),
            },
        );
        ParsedSchema {
            query_type: Some("Query".to_string()),
            mutation_type: None,
            subscription_type: None,
            types,
            directives: Vec::new(),
        }
    }

    /// The emitted SDL has to parse. It did not: a description carrying a
    /// quote was interpolated raw, and every conforming parser rejected the
    /// result — which is how a 597KB schema came to be unreadable.
    #[test]
    fn a_description_with_quotes_still_emits_parseable_sdl() {
        let schema =
            schema_with_description(r#"The MIME type of the content (e.g., "text/plain")."#);
        let sdl = generate_sdl(&schema);
        assert!(
            sdl.contains(r#"\"text/plain\""#),
            "the quotes should be escaped, got:\n{sdl}"
        );
        assert!(
            async_graphql_parser::parse_schema(&sdl).is_ok(),
            "emitted SDL must parse:\n{sdl}"
        );
    }

    #[test]
    fn a_description_with_a_backslash_survives() {
        let schema = schema_with_description(r"A Windows path like C:\\Users");
        let sdl = generate_sdl(&schema);
        assert!(
            async_graphql_parser::parse_schema(&sdl).is_ok(),
            "emitted SDL must parse:\n{sdl}"
        );
    }

    #[test]
    fn a_description_with_control_characters_survives() {
        let schema = schema_with_description("tab\there and bell\u{7}there");
        let sdl = generate_sdl(&schema);
        assert!(
            async_graphql_parser::parse_schema(&sdl).is_ok(),
            "emitted SDL must parse:\n{sdl}"
        );
    }

    #[test]
    fn a_multiline_description_containing_a_block_delimiter_survives() {
        let schema = schema_with_description("first line\nhas a \"\"\" inside\nlast line");
        let sdl = generate_sdl(&schema);
        assert!(
            async_graphql_parser::parse_schema(&sdl).is_ok(),
            "emitted SDL must parse:\n{sdl}"
        );
    }

    #[test]
    fn the_description_still_says_what_it_said() {
        let original = r#"The MIME type (e.g., "text/plain")."#;
        let sdl = generate_sdl(&schema_with_description(original));
        let reparsed = crate::spec::infer::graphql::parse_sdl(&sdl).unwrap();
        let field = reparsed
            .types
            .get("Thing")
            .unwrap()
            .fields
            .iter()
            .find(|f| f.name == "mediaType")
            .unwrap();
        assert_eq!(
            field.description.as_deref(),
            Some(original),
            "escaping must be reversible, not lossy"
        );
    }
}
