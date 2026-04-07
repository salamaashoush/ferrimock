//! GraphQL schema parser for introspection responses

use super::types::*;
use anyhow::{Context, Result};
use rustc_hash::FxHashMap;

/// Parser for GraphQL introspection responses
pub struct SchemaParser;

impl SchemaParser {
    /// Parse an introspection response into a structured schema
    ///
    /// # Errors
    /// Returns an error if the introspection response cannot be parsed
    pub fn parse(response: IntrospectionResponse) -> Result<ParsedSchema> {
        let schema_intro = response.data.schema;

        // Extract root operation type names
        let query_type = schema_intro.query_type.map(|t| t.name);
        let mutation_type = schema_intro.mutation_type.map(|t| t.name);
        let subscription_type = schema_intro.subscription_type.map(|t| t.name);

        // Parse all types into a map
        let mut types = FxHashMap::default();
        for full_type in schema_intro.types {
            if let Some(type_def) = Self::parse_type(full_type)? {
                types.insert(type_def.name.clone(), type_def);
            }
        }

        // Parse directives
        let directives = schema_intro
            .directives
            .into_iter()
            .map(Self::parse_directive)
            .collect();

        Ok(ParsedSchema {
            query_type,
            mutation_type,
            subscription_type,
            types,
            directives,
        })
    }

    /// Parse a single type from introspection
    fn parse_type(full_type: FullType) -> Result<Option<TypeDefinition>> {
        // Skip if no name (wrapper types)
        let Some(name) = full_type.name else {
            return Ok(None);
        };

        // Parse type kind
        let kind = TypeKind::parse(&full_type.kind)
            .with_context(|| format!("Unknown type kind: {}", full_type.kind))?;

        // Parse fields
        let fields = full_type
            .fields
            .unwrap_or_default()
            .into_iter()
            .map(Self::parse_field)
            .collect();

        // Parse input fields
        let input_fields = full_type
            .input_fields
            .unwrap_or_default()
            .into_iter()
            .map(Self::parse_input_value)
            .collect();

        // Parse interfaces
        let interfaces = full_type
            .interfaces
            .unwrap_or_default()
            .into_iter()
            .map(|i| TypeRef::from_introspection(&i))
            .collect();

        // Parse enum values
        let enum_values = full_type
            .enum_values
            .unwrap_or_default()
            .into_iter()
            .map(Self::parse_enum_value)
            .collect();

        // Parse possible types (for unions/interfaces)
        let possible_types = full_type
            .possible_types
            .unwrap_or_default()
            .into_iter()
            .map(|t| TypeRef::from_introspection(&t))
            .collect();

        Ok(Some(TypeDefinition {
            kind,
            name,
            description: full_type.description,
            fields,
            input_fields,
            interfaces,
            enum_values,
            possible_types,
        }))
    }

    /// Parse a field definition
    fn parse_field(field: FieldIntrospection) -> FieldDefinition {
        FieldDefinition {
            name: field.name,
            description: field.description,
            args: field
                .args
                .into_iter()
                .map(Self::parse_input_value)
                .collect(),
            field_type: TypeRef::from_introspection(&field.field_type),
            is_deprecated: field.is_deprecated,
            deprecation_reason: field.deprecation_reason,
        }
    }

    /// Parse an input value (argument or input field)
    fn parse_input_value(input: InputValueIntrospection) -> InputValueDefinition {
        InputValueDefinition {
            name: input.name,
            description: input.description,
            value_type: TypeRef::from_introspection(&input.value_type),
            default_value: input.default_value,
        }
    }

    /// Parse an enum value
    fn parse_enum_value(value: EnumValueIntrospection) -> EnumValueDefinition {
        EnumValueDefinition {
            name: value.name,
            description: value.description,
            is_deprecated: value.is_deprecated,
            deprecation_reason: value.deprecation_reason,
        }
    }

    /// Parse a directive
    fn parse_directive(directive: DirectiveIntrospection) -> DirectiveDefinition {
        DirectiveDefinition {
            name: directive.name,
            description: directive.description,
            locations: directive.locations,
            args: directive
                .args
                .into_iter()
                .map(Self::parse_input_value)
                .collect(),
        }
    }
}

impl ParsedSchema {
    /// Get all operations for a specific operation type
    pub fn get_operations(&self, operation_type: OperationType) -> Vec<&FieldDefinition> {
        let type_name = match operation_type {
            OperationType::Query => &self.query_type,
            OperationType::Mutation => &self.mutation_type,
            OperationType::Subscription => &self.subscription_type,
        };

        if let Some(name) = type_name {
            if let Some(type_def) = self.types.get(name) {
                return type_def.fields.iter().collect();
            }
        }

        vec![]
    }

    /// Get a type by name
    pub fn get_type(&self, name: &str) -> Option<&TypeDefinition> {
        self.types.get(name)
    }

    /// Check if a type is a built-in scalar
    pub fn is_builtin_scalar(name: &str) -> bool {
        matches!(name, "ID" | "String" | "Int" | "Float" | "Boolean")
    }

    /// Get all user-defined types (excluding built-ins and introspection types)
    pub fn get_user_types(&self) -> Vec<&TypeDefinition> {
        self.types
            .values()
            .filter(|t| !t.name.starts_with("__") && !Self::is_builtin_scalar(&t.name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_simple_schema() {
        let response_json = json!({
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
                      "description": "Get a user by ID",
                      "args": [
                        {
                          "name": "id",
                          "description": "User ID",
                          "type": {
                            "kind": "NON_NULL",
                            "name": null,
                            "ofType": {
                              "kind": "SCALAR",
                              "name": "ID",
                              "ofType": null
                            }
                          },
                          "defaultValue": null
                        }
                      ],
                      "type": {
                        "kind": "OBJECT",
                        "name": "User",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "OBJECT",
                  "name": "User",
                  "description": "A user",
                  "fields": [
                    {
                      "name": "id",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "NON_NULL",
                        "name": null,
                        "ofType": {
                          "kind": "SCALAR",
                          "name": "ID",
                          "ofType": null
                        }
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    },
                    {
                      "name": "name",
                      "description": null,
                      "args": [],
                      "type": {
                        "kind": "SCALAR",
                        "name": "String",
                        "ofType": null
                      },
                      "isDeprecated": false,
                      "deprecationReason": null
                    }
                  ],
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "ID",
                  "description": "The ID scalar type",
                  "fields": null,
                  "inputFields": null,
                  "interfaces": null,
                  "enumValues": null,
                  "possibleTypes": null
                },
                {
                  "kind": "SCALAR",
                  "name": "String",
                  "description": "The String scalar type",
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

        let response: IntrospectionResponse = serde_json::from_value(response_json)
            .expect("Failed to deserialize introspection response");
        let schema = SchemaParser::parse(response).expect("Failed to parse GraphQL schema");

        assert_eq!(schema.query_type, Some("Query".to_string()));
        assert_eq!(schema.mutation_type, None);
        assert_eq!(schema.subscription_type, None);
        assert_eq!(schema.types.len(), 4);

        // Check Query type
        let query_type = schema
            .get_type("Query")
            .expect("Failed to get Query type from schema");
        assert_eq!(query_type.kind, TypeKind::Object);
        assert_eq!(query_type.fields.len(), 1);
        assert_eq!(query_type.fields[0].name, "user");

        // Check User type
        let user_type = schema
            .get_type("User")
            .expect("Failed to get User type from schema");
        assert_eq!(user_type.kind, TypeKind::Object);
        assert_eq!(user_type.fields.len(), 2);

        // Check operations
        let queries = schema.get_operations(OperationType::Query);
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].name, "user");
    }

    #[test]
    fn test_type_ref_unwrapping() {
        // Test NON_NULL unwrapping
        let non_null_ref = TypeRefIntrospection {
            kind: "NON_NULL".to_string(),
            name: None,
            of_type: Some(Box::new(TypeRefIntrospection {
                kind: "SCALAR".to_string(),
                name: Some("String".to_string()),
                of_type: None,
            })),
        };

        let type_ref = TypeRef::from_introspection(&non_null_ref);
        assert_eq!(type_ref.name, "String");
        assert!(type_ref.is_non_null);
        assert!(!type_ref.is_list);

        // Test LIST unwrapping
        let list_ref = TypeRefIntrospection {
            kind: "LIST".to_string(),
            name: None,
            of_type: Some(Box::new(TypeRefIntrospection {
                kind: "SCALAR".to_string(),
                name: Some("Int".to_string()),
                of_type: None,
            })),
        };

        let type_ref = TypeRef::from_introspection(&list_ref);
        assert_eq!(type_ref.name, "Int");
        assert!(!type_ref.is_non_null);
        assert!(type_ref.is_list);

        // Test NON_NULL LIST
        let non_null_list_ref = TypeRefIntrospection {
            kind: "NON_NULL".to_string(),
            name: None,
            of_type: Some(Box::new(TypeRefIntrospection {
                kind: "LIST".to_string(),
                name: None,
                of_type: Some(Box::new(TypeRefIntrospection {
                    kind: "SCALAR".to_string(),
                    name: Some("String".to_string()),
                    of_type: None,
                })),
            })),
        };

        let type_ref = TypeRef::from_introspection(&non_null_list_ref);
        assert_eq!(type_ref.name, "String");
        assert!(type_ref.is_non_null);
        assert!(type_ref.is_list);
    }
}
