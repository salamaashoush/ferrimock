//! Mock generator from GraphQL schema

use super::introspection::{
    FieldDefinition, OperationType, ParsedSchema, TypeDefinition, TypeKind, TypeRef,
};
use super::type_mapper::TypeToFakeMapper;
use anyhow::{Context, Result};
use crate::config::{
    GraphQLMatchConfig, MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Options for mock generation
#[derive(Debug, Clone)]
pub struct GeneratorOptions {
    /// Generate mocks for queries
    pub include_queries: bool,
    /// Generate mocks for mutations
    pub include_mutations: bool,
    /// Generate mocks for subscriptions
    pub include_subscriptions: bool,
    /// Base priority for generated mocks
    pub base_priority: u32,
    /// Include deprecated fields in responses
    pub include_deprecated: bool,
    /// Generate variant mocks with different variables
    pub generate_variants: bool,
    /// Default length for array/list fields
    pub default_list_length: usize,
    /// Maximum nesting depth to prevent infinite recursion
    pub max_depth: usize,
    /// GraphQL endpoint URL path (e.g., "/graphql" or "/app-api/graphql")
    pub endpoint_url: String,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            include_queries: true,
            include_mutations: true,
            include_subscriptions: false,
            base_priority: 100,
            include_deprecated: false,
            generate_variants: false,
            default_list_length: 3,
            max_depth: 5,
            endpoint_url: "/graphql".to_string(),
        }
    }
}

/// Generates mock configurations from a GraphQL schema
pub struct MockGenerator {
    schema: ParsedSchema,
    options: GeneratorOptions,
    pub type_mapper: TypeToFakeMapper,
}

impl MockGenerator {
    /// Create a new mock generator
    pub fn new(schema: ParsedSchema, options: GeneratorOptions) -> Self {
        Self {
            schema,
            options,
            type_mapper: TypeToFakeMapper::new(),
        }
    }

    /// Generate all mocks from the schema
    pub fn generate_all(&self) -> Result<MockCollectionConfig> {
        let mut mocks = Vec::new();

        if self.options.include_queries {
            mocks.extend(self.generate_query_mocks()?);
        }

        if self.options.include_mutations {
            mocks.extend(self.generate_mutation_mocks()?);
        }

        if self.options.include_subscriptions {
            mocks.extend(self.generate_subscription_mocks()?);
        }

        Ok(MockCollectionConfig {
            name: Some("Auto-generated from GraphQL Schema".to_string()),
            description: Some("Generated via introspection query".to_string()),
            enabled: true,
            vars: None,
            mocks,
        })
    }

    /// Generate mocks for all queries
    fn generate_query_mocks(&self) -> Result<Vec<MockConfig>> {
        self.generate_operation_mocks(OperationType::Query)
    }

    /// Generate mocks for all mutations
    fn generate_mutation_mocks(&self) -> Result<Vec<MockConfig>> {
        self.generate_operation_mocks(OperationType::Mutation)
    }

    /// Generate mocks for all subscriptions
    fn generate_subscription_mocks(&self) -> Result<Vec<MockConfig>> {
        self.generate_operation_mocks(OperationType::Subscription)
    }

    /// Generate mocks for a specific operation type
    fn generate_operation_mocks(&self, op_type: OperationType) -> Result<Vec<MockConfig>> {
        let operations = self.schema.get_operations(op_type);
        let mut mocks = Vec::new();

        for operation in operations {
            let mock = self.generate_operation_mock(operation, op_type)?;
            mocks.push(mock);
        }

        Ok(mocks)
    }

    /// Extract variable names from operation arguments
    fn extract_variable_names(operation: &FieldDefinition) -> Vec<String> {
        operation.args.iter().map(|arg| arg.name.clone()).collect()
    }

    /// Check if a field name matches a variable name (handles nested paths)
    fn matches_variable(field_name: &str, variables: &[String]) -> Option<String> {
        // Direct match
        if variables.contains(&field_name.to_string()) {
            return Some(format!("{{{{ body_json.variables.{field_name} }}}}"));
        }

        // Check nested paths: data.user.id -> match "id" variable
        let field_parts: Vec<&str> = field_name.split('.').collect();
        if let Some(last_part) = field_parts.last() {
            let clean_field = last_part.trim_start_matches('[').trim_end_matches(']');
            if variables.contains(&clean_field.to_string()) {
                return Some(format!("{{{{ body_json.variables.{clean_field} }}}}"));
            }
        }

        // Check if field ends with variable name: userId -> match "id" variable
        for var in variables {
            if field_name.to_lowercase().ends_with(&var.to_lowercase()) && field_name != var {
                // Check if it's a reasonable match (not just coincidence)
                if field_name.len() - var.len() <= 10 {
                    return Some(format!("{{{{ body_json.variables.{var} }}}}"));
                }
            }
        }

        None
    }

    /// Check if variables contain pagination-related params
    fn has_pagination_variable(variables: &[String], param_names: &[&str]) -> Option<String> {
        for param in param_names {
            if variables.contains(&param.to_string()) {
                return Some(param.to_string());
            }
        }
        None
    }

    /// Detect if a field is a pagination-related field and should use variable
    fn is_pagination_field(field_name: &str) -> bool {
        matches!(
            field_name,
            "hasNextPage" | "hasPreviousPage" | "totalCount" | "startCursor" | "endCursor"
        )
    }

    /// Generate a mock for a single operation
    fn generate_operation_mock(
        &self,
        operation: &FieldDefinition,
        op_type: OperationType,
    ) -> Result<MockConfig> {
        let mock_id = format!("{}-{}", op_type.as_str(), operation.name);

        // Extract variable names from operation arguments
        let variables = Self::extract_variable_names(operation);

        // Build GraphQL matcher config
        let graphql_config = match op_type {
            OperationType::Query => GraphQLMatchConfig::Structured {
                operation: Some(operation.name.clone()),
                query: Some(operation.name.clone()),
                mutation: None,
                subscription: None,
                introspection: None,
                variables: FxHashMap::default(),
            },
            OperationType::Mutation => GraphQLMatchConfig::Structured {
                operation: Some(operation.name.clone()),
                query: None,
                mutation: Some(operation.name.clone()),
                subscription: None,
                introspection: None,
                variables: FxHashMap::default(),
            },
            OperationType::Subscription => GraphQLMatchConfig::Structured {
                operation: Some(operation.name.clone()),
                query: None,
                mutation: None,
                subscription: Some(operation.name.clone()),
                introspection: None,
                variables: FxHashMap::default(),
            },
        };

        // Generate response template with variable support
        let response_template =
            self.generate_response_template_with_variables(operation, &variables)?;

        Ok(MockConfig {
            id: mock_id.into(),
            description: None,
            priority: self.options.base_priority,
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                method: None,
                methods: vec!["POST".to_string()],
                url: Some(self.options.endpoint_url.clone()),
                urls: vec![],
                headers: FxHashMap::default(),
                query: FxHashMap::default(),
                body: FxHashMap::default(),
                graphql: Some(graphql_config),
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())]
                    .into_iter()
                    .collect(),
                body: Some(response_template),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: None,
        })
    }

    /// Generate Tera template for operation response with variable support
    fn generate_response_template_with_variables(
        &self,
        operation: &FieldDefinition,
        variables: &[String],
    ) -> Result<String> {
        let mut template = String::from("{\n  \"data\": {\n    \"");
        template.push_str(&operation.name);
        template.push_str("\": ");

        // Track visited types to detect circular references
        let mut visited = FxHashSet::default();

        // Generate nested field structure with variable support
        template.push_str(&self.generate_type_template_with_variables(
            &operation.field_type,
            None,
            2,
            &mut visited,
            variables,
        )?);

        template.push_str("\n  }\n}");
        Ok(template)
    }

    /// Recursively generate template for a type with variable support
    fn generate_type_template_with_variables(
        &self,
        type_ref: &TypeRef,
        field_name: Option<&str>,
        depth: usize,
        visited: &mut FxHashSet<String>,
        variables: &[String],
    ) -> Result<String> {
        // Check depth limit
        if depth > self.options.max_depth {
            return Ok("null".to_string());
        }

        let unwrapped = type_ref.unwrap();

        // Check for circular reference
        if visited.contains(&unwrapped.name) {
            return Ok("null".to_string());
        }

        // PRIORITY 1: Check if field matches a GraphQL variable
        if let Some(name) = field_name
            && let Some(var_expr) = Self::matches_variable(name, variables)
        {
            // Wrap in array if this is a list type
            if unwrapped.is_list {
                let indent = "  ".repeat(depth);
                return Ok(format!(
                    "[\n{}{}\n{}]",
                    "  ".repeat(depth + 1),
                    var_expr,
                    indent
                ));
            }
            return Ok(var_expr);
        }

        // Get the type definition
        let type_def = self
            .schema
            .get_type(&unwrapped.name)
            .with_context(|| format!("Unknown type: {}", unwrapped.name))?;

        // Generate template based on type kind
        let single_value = match type_def.kind {
            TypeKind::Scalar => {
                use crate::codegen::field_type_to_tera_expr;
                use crate::type_detector::{
                    detect_from_field_name_only, detect_from_semantic_context,
                };

                if let Some(name) = field_name {
                    // Special handling for pagination fields
                    if Self::is_pagination_field(name) {
                        match name {
                            "hasNextPage" => {
                                // Smart hasNextPage: true if 'after' variable exists, false otherwise
                                if Self::has_pagination_variable(variables, &["after", "cursor"])
                                    .is_some()
                                {
                                    return Ok("{{ fake_boolean() }}".to_string());
                                }
                                return Ok("false".to_string());
                            }
                            "hasPreviousPage" => {
                                // Smart hasPreviousPage: true if 'before' variable exists
                                if Self::has_pagination_variable(variables, &["before"]).is_some() {
                                    return Ok("{{ fake_boolean() }}".to_string());
                                }
                                return Ok("false".to_string());
                            }
                            _ => {} // Continue with normal detection for other pagination fields
                        }
                    }

                    // Try name-only detection
                    if let Some((field_type, _confidence)) = detect_from_field_name_only(name) {
                        return Ok(field_type_to_tera_expr(name, &field_type, false));
                    }

                    // Try full semantic detection
                    if let Some((field_type, _confidence)) = detect_from_semantic_context(name, &[])
                    {
                        return Ok(field_type_to_tera_expr(name, &field_type, false));
                    }

                    self.type_mapper.scalar_to_fake(&unwrapped.name)
                } else {
                    self.type_mapper.scalar_to_fake(&unwrapped.name)
                }
            }
            TypeKind::Enum => {
                let enum_values: Vec<String> = type_def
                    .enum_values
                    .iter()
                    .map(|v| v.name.clone())
                    .collect();
                self.type_mapper.enum_to_fake(&enum_values)
            }
            TypeKind::Object => {
                visited.insert(unwrapped.name.clone());
                let result = self
                    .generate_object_template_with_variables(type_def, depth, visited, variables)?;
                visited.remove(&unwrapped.name);
                result
            }
            TypeKind::Interface | TypeKind::Union => {
                if type_def.possible_types.is_empty() {
                    return Ok("null".to_string());
                }

                visited.insert(unwrapped.name.clone());

                // Generate smart union/interface with random type selection
                let result =
                    self.generate_union_or_interface_template(type_def, depth, visited, variables)?;

                visited.remove(&unwrapped.name);
                result
            }
            _ => "null".to_string(),
        };

        // Wrap in array if this is a list type
        if unwrapped.is_list {
            let indent = "  ".repeat(depth);
            let inner_indent = "  ".repeat(depth + 1);

            let mut array_template = String::new();

            // Smart array length: Use pagination variables if available
            // Use {% set %} statement to avoid nested {{ }} expressions in {% for %}
            if let Some(param) = Self::has_pagination_variable(variables, &["first", "limit"]) {
                array_template.push_str(&indent);
                array_template.push_str("{% set __array_length = body_json.variables.");
                array_template.push_str(&param);
                array_template.push_str(" | default(value=");
                array_template.push_str(&self.options.default_list_length.to_string());
                array_template.push_str(") %}\n");
                array_template.push_str(&indent);
                array_template.push_str("[\n");
                array_template.push_str(&inner_indent);
                array_template.push_str("{% for i in range(start=0, end=__array_length) %}\n");
            } else if Self::has_pagination_variable(variables, &["last"]).is_some() {
                array_template.push_str(&indent);
                array_template
                    .push_str("{% set __array_length = body_json.variables.last | default(value=");
                array_template.push_str(&self.options.default_list_length.to_string());
                array_template.push_str(") %}\n");
                array_template.push_str(&indent);
                array_template.push_str("[\n");
                array_template.push_str(&inner_indent);
                array_template.push_str("{% for i in range(start=0, end=__array_length) %}\n");
            } else {
                array_template.push_str("[\n");
                array_template.push_str(&inner_indent);
                array_template.push_str("{% for i in range(start=0, end=");
                array_template.push_str(&self.options.default_list_length.to_string());
                array_template.push_str(") %}\n");
            }

            array_template.push_str(&inner_indent);
            array_template.push_str(&single_value);
            array_template.push_str("{% if not loop.last %},{% endif %}\n");
            array_template.push_str(&inner_indent);
            array_template.push_str("{% endfor %}\n");
            array_template.push_str(&indent);
            array_template.push(']');

            Ok(array_template)
        } else {
            Ok(single_value)
        }
    }

    /// Generate template for an object type with variable support
    fn generate_object_template_with_variables(
        &self,
        type_def: &TypeDefinition,
        depth: usize,
        visited: &mut FxHashSet<String>,
        variables: &[String],
    ) -> Result<String> {
        let indent = "  ".repeat(depth);
        let field_indent = "  ".repeat(depth + 1);

        let mut template = String::from("{\n");

        let mut field_count = 0;
        for field in &type_def.fields {
            if field.is_deprecated && !self.options.include_deprecated {
                continue;
            }

            if !self.should_include_field(field, depth) {
                continue;
            }

            if field_count > 0 {
                template.push_str(",\n");
            }

            template.push_str(&field_indent);
            template.push('"');
            template.push_str(&field.name);
            template.push_str("\": ");

            let field_template = self.generate_type_template_with_variables(
                &field.field_type,
                Some(&field.name),
                depth + 1,
                visited,
                variables,
            )?;
            template.push_str(&field_template);

            field_count += 1;
        }

        if field_count > 0 {
            template.push('\n');
        }
        template.push_str(&indent);
        template.push('}');

        Ok(template)
    }

    /// Determine if a field should be included in the generated mock
    fn should_include_field(&self, field: &FieldDefinition, depth: usize) -> bool {
        // Always include ID fields
        if field.name == "id" {
            return true;
        }

        // Skip complex nested objects if we're already deep
        if depth > 3 && self.is_complex_type(&field.field_type) {
            return false;
        }

        // Include commonly used fields
        if matches!(
            field.name.as_str(),
            "name"
                | "title"
                | "description"
                | "email"
                | "status"
                | "type"
                | "kind"
                | "createdAt"
                | "updatedAt"
        ) {
            return true;
        }

        // Include all fields by default unless we're very deep
        depth <= 4
    }

    /// Check if a type is complex (nested object/interface/union)
    fn is_complex_type(&self, type_ref: &TypeRef) -> bool {
        let unwrapped = type_ref.unwrap();

        if let Some(type_def) = self.schema.get_type(&unwrapped.name) {
            matches!(
                type_def.kind,
                TypeKind::Object | TypeKind::Interface | TypeKind::Union
            )
        } else {
            false
        }
    }

    /// Generate template for Union or Interface with random type selection and __typename
    fn generate_union_or_interface_template(
        &self,
        type_def: &TypeDefinition,
        depth: usize,
        visited: &mut FxHashSet<String>,
        variables: &[String],
    ) -> Result<String> {
        let possible_types = &type_def.possible_types;

        if possible_types.len() == 1 {
            // Only one possible type - generate it directly with __typename
            let type_ref = possible_types
                .first()
                .context("Expected at least one possible type")?;
            let concrete_type = self
                .schema
                .get_type(&type_ref.name)
                .with_context(|| format!("Unknown type: {}", type_ref.name))?;

            return self.generate_object_with_typename(
                concrete_type,
                &type_ref.name,
                depth,
                visited,
                variables,
            );
        }

        // Multiple possible types - use Tera random selection
        let indent = "  ".repeat(depth);

        let mut template = String::from("{% set __union_type = [");

        // Build array of type names for random selection
        let type_names: Vec<String> = possible_types
            .iter()
            .map(|t| format!("\"{}\"", t.name))
            .collect();
        template.push_str(&type_names.join(", "));
        template.push_str("] | random_choice %}\n");
        template.push_str(&indent);

        // Generate conditional branches for each possible type
        for (i, type_ref) in possible_types.iter().enumerate() {
            let concrete_type = self
                .schema
                .get_type(&type_ref.name)
                .with_context(|| format!("Unknown type: {}", type_ref.name))?;

            if i == 0 {
                template.push_str("{% if __union_type == \"");
            } else {
                template.push_str("{% elif __union_type == \"");
            }
            template.push_str(&type_ref.name);
            template.push_str("\" %}\n");
            template.push_str(&indent);

            // Generate the object template for this type with __typename
            let obj_template = self.generate_object_with_typename(
                concrete_type,
                &type_ref.name,
                depth,
                visited,
                variables,
            )?;
            template.push_str(&obj_template);
            template.push('\n');
            template.push_str(&indent);
        }

        template.push_str("{% endif %}");

        Ok(template)
    }

    /// Generate object template with __typename field injected
    fn generate_object_with_typename(
        &self,
        type_def: &TypeDefinition,
        typename: &str,
        depth: usize,
        visited: &mut FxHashSet<String>,
        variables: &[String],
    ) -> Result<String> {
        let indent = "  ".repeat(depth);
        let field_indent = "  ".repeat(depth + 1);

        let mut template = String::from("{\n");

        // Inject __typename as first field
        template.push_str(&field_indent);
        template.push_str("\"__typename\": \"");
        template.push_str(typename);
        template.push('"');

        // Generate other fields
        let mut has_fields = false;
        for field in &type_def.fields {
            if field.is_deprecated && !self.options.include_deprecated {
                continue;
            }

            if !self.should_include_field(field, depth) {
                continue;
            }

            template.push_str(",\n");
            template.push_str(&field_indent);
            template.push('"');
            template.push_str(&field.name);
            template.push_str("\": ");

            let field_template = self.generate_type_template_with_variables(
                &field.field_type,
                Some(&field.name),
                depth + 1,
                visited,
                variables,
            )?;
            template.push_str(&field_template);

            has_fields = true;
        }

        if has_fields {
            template.push('\n');
        }
        template.push_str(&indent);
        template.push('}');

        Ok(template)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generator_options_default() {
        let options = GeneratorOptions::default();
        assert!(options.include_queries);
        assert!(options.include_mutations);
        assert!(!options.include_subscriptions);
        assert_eq!(options.base_priority, 100);
        assert_eq!(options.default_list_length, 3);
        assert_eq!(options.max_depth, 5);
    }
}
