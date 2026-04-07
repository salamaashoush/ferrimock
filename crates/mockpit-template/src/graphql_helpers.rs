//! GraphQL-specific template helper functions

// Tera library callbacks require std::collections::HashMap - cannot use FxHashMap
#![allow(clippy::disallowed_types)]

use serde_json::{Map, Value};
use std::collections::HashMap;
use tera::{Error, Result};

/// Create GraphQL error response
///
/// Usage: `{{ graphql_error(message="User not found", code="NOT_FOUND") }}`
pub fn graphql_error<S: ::std::hash::BuildHasher>(
    args: &HashMap<String, Value, S>,
) -> Result<Value> {
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::msg("graphql_error requires 'message' argument"))?;

    let code = args.get("code").and_then(|v| v.as_str());
    let path = args.get("path").and_then(|v| v.as_array());

    let mut error = Map::new();
    error.insert("message".to_string(), Value::String(message.to_string()));

    if let Some(code_val) = code {
        let mut extensions = Map::new();
        extensions.insert("code".to_string(), Value::String(code_val.to_string()));
        error.insert("extensions".to_string(), Value::Object(extensions));
    }

    if let Some(path_val) = path {
        error.insert("path".to_string(), Value::Array(path_val.clone()));
    }

    let mut result = Map::new();
    result.insert(
        "errors".to_string(),
        Value::Array(vec![Value::Object(error)]),
    );
    result.insert("data".to_string(), Value::Null);

    Ok(Value::Object(result))
}

/// Create GraphQL field error (with automatic path parsing)
///
/// Usage: `{{ graphql_field_error(field="user.email", message="Invalid email", code="VALIDATION_ERROR") }}`
pub fn graphql_field_error<S: ::std::hash::BuildHasher>(
    args: &HashMap<String, Value, S>,
) -> Result<Value> {
    let field = args
        .get("field")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::msg("graphql_field_error requires 'field' argument"))?;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::msg("graphql_field_error requires 'message' argument"))?;

    let code = args.get("code").and_then(|v| v.as_str());

    let path: Vec<Value> = field
        .split('.')
        .map(|s| Value::String(s.to_string()))
        .collect();

    let mut error = Map::new();
    error.insert("message".to_string(), Value::String(message.to_string()));
    error.insert("path".to_string(), Value::Array(path));

    if let Some(code_val) = code {
        let mut extensions = Map::new();
        extensions.insert("code".to_string(), Value::String(code_val.to_string()));
        error.insert("extensions".to_string(), Value::Object(extensions));
    }

    let mut result = Map::new();
    result.insert(
        "errors".to_string(),
        Value::Array(vec![Value::Object(error)]),
    );
    result.insert("data".to_string(), Value::Null);

    Ok(Value::Object(result))
}

/// Build a simple GraphQL type for __type introspection queries
///
/// Usage: `{{ graphql_type(name="User", kind="OBJECT") }}`
pub fn graphql_type<S: ::std::hash::BuildHasher>(
    args: &HashMap<String, Value, S>,
) -> Result<Value> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::msg("graphql_type requires 'name'"))?;

    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("OBJECT");
    let description = args.get("description").and_then(|v| v.as_str());
    let fields = args.get("fields");

    let mut type_def = Map::new();
    type_def.insert("name".to_string(), Value::String(name.to_string()));
    type_def.insert("kind".to_string(), Value::String(kind.to_string()));

    if let Some(desc) = description {
        type_def.insert("description".to_string(), Value::String(desc.to_string()));
    }

    if let Some(fields_val) = fields {
        type_def.insert("fields".to_string(), fields_val.clone());
    }

    let mut type_wrapper = Map::new();
    type_wrapper.insert("__type".to_string(), Value::Object(type_def));

    let mut result = Map::new();
    result.insert("data".to_string(), Value::Object(type_wrapper));

    Ok(Value::Object(result))
}

/// Build a GraphQL schema introspection response
///
/// Usage: `{{ graphql_schema(types=[...]) }}` or just `{{ graphql_schema() }}` for empty schema
pub fn graphql_schema<S: ::std::hash::BuildHasher>(
    args: &HashMap<String, Value, S>,
) -> Result<Value> {
    let query_type = args
        .get("queryType")
        .and_then(|v| v.as_str())
        .unwrap_or("Query");
    let mutation_type = args.get("mutationType").and_then(|v| v.as_str());
    let subscription_type = args.get("subscriptionType").and_then(|v| v.as_str());
    let types = args.get("types");

    let mut query_type_map = Map::new();
    query_type_map.insert("name".to_string(), Value::String(query_type.to_string()));

    let mutation_type_value = if let Some(n) = mutation_type {
        let mut mutation_map = Map::new();
        mutation_map.insert("name".to_string(), Value::String(n.to_string()));
        Value::Object(mutation_map)
    } else {
        Value::Null
    };

    let subscription_type_value = if let Some(n) = subscription_type {
        let mut subscription_map = Map::new();
        subscription_map.insert("name".to_string(), Value::String(n.to_string()));
        Value::Object(subscription_map)
    } else {
        Value::Null
    };

    let mut schema = Map::new();
    schema.insert("queryType".to_string(), Value::Object(query_type_map));
    schema.insert("mutationType".to_string(), mutation_type_value);
    schema.insert("subscriptionType".to_string(), subscription_type_value);

    if let Some(types_val) = types {
        schema.insert("types".to_string(), types_val.clone());
    }

    let mut schema_wrapper = Map::new();
    schema_wrapper.insert("__schema".to_string(), Value::Object(schema));

    let mut result = Map::new();
    result.insert("data".to_string(), Value::Object(schema_wrapper));

    Ok(Value::Object(result))
}

/// Register all GraphQL helper functions with Tera
pub fn register_all_functions(tera: &mut tera::Tera) {
    tera.register_function("graphql_error", graphql_error);
    tera.register_function("graphql_field_error", graphql_field_error);
    tera.register_function("graphql_type", graphql_type);
    tera.register_function("graphql_schema", graphql_schema);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_graphql_error() {
        let mut args: HashMap<String, Value> = HashMap::new();
        args.insert(
            "message".to_string(),
            Value::String("Not found".to_string()),
        );
        args.insert("code".to_string(), Value::String("NOT_FOUND".to_string()));

        let result = graphql_error(&args).expect("Failed to create GraphQL error");
        assert_eq!(result["errors"][0]["message"], "Not found");
        assert_eq!(result["errors"][0]["extensions"]["code"], "NOT_FOUND");
        assert_eq!(result["data"], Value::Null);
    }

    #[test]
    fn test_graphql_field_error() {
        let mut args: HashMap<String, Value> = HashMap::new();
        args.insert("field".to_string(), Value::String("user.email".to_string()));
        args.insert(
            "message".to_string(),
            Value::String("Invalid email".to_string()),
        );
        args.insert(
            "code".to_string(),
            Value::String("VALIDATION_ERROR".to_string()),
        );

        let result = graphql_field_error(&args).expect("Failed to create GraphQL field error");
        assert_eq!(result["errors"][0]["message"], "Invalid email");
        assert_eq!(result["errors"][0]["path"][0], "user");
        assert_eq!(result["errors"][0]["path"][1], "email");
        assert_eq!(
            result["errors"][0]["extensions"]["code"],
            "VALIDATION_ERROR"
        );
    }

    #[test]
    fn test_graphql_type() {
        let mut args: HashMap<String, Value> = HashMap::new();
        args.insert("name".to_string(), Value::String("User".to_string()));
        args.insert("kind".to_string(), Value::String("OBJECT".to_string()));

        let result = graphql_type(&args).expect("Failed to create GraphQL type");
        assert_eq!(result["data"]["__type"]["name"], "User");
        assert_eq!(result["data"]["__type"]["kind"], "OBJECT");
    }

    #[test]
    fn test_graphql_schema() {
        let mut args: HashMap<String, Value> = HashMap::new();
        args.insert("queryType".to_string(), Value::String("Query".to_string()));
        args.insert(
            "mutationType".to_string(),
            Value::String("Mutation".to_string()),
        );

        let result = graphql_schema(&args).expect("Failed to create GraphQL schema");
        assert_eq!(result["data"]["__schema"]["queryType"]["name"], "Query");
        assert_eq!(
            result["data"]["__schema"]["mutationType"]["name"],
            "Mutation"
        );
        assert_eq!(result["data"]["__schema"]["subscriptionType"], Value::Null);
    }
}
