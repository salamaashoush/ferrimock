//! Standard GraphQL introspection query
//!
//! This module provides the standard GraphQL introspection query used to
//! discover schema information from a GraphQL endpoint.

/// Get the standard GraphQL introspection query
///
/// This query fetches complete schema information including:
/// - Query, Mutation, and Subscription root types
/// - All types with their fields, arguments, and descriptions
/// - Interfaces and their implementations
/// - Unions and their possible types
/// - Enums and their values
/// - Input types and their fields
/// - Directives and their locations
///
/// The query includes deprecated fields and enum values by using
/// `includeDeprecated: true` parameter.
pub fn get_introspection_query() -> &'static str {
    r"
    query IntrospectionQuery {
      __schema {
        queryType { name }
        mutationType { name }
        subscriptionType { name }
        types {
          ...FullType
        }
        directives {
          name
          description
          locations
          args {
            ...InputValue
          }
        }
      }
    }

    fragment FullType on __Type {
      kind
      name
      description
      fields(includeDeprecated: true) {
        name
        description
        args {
          ...InputValue
        }
        type {
          ...TypeRef
        }
        isDeprecated
        deprecationReason
      }
      inputFields {
        ...InputValue
      }
      interfaces {
        ...TypeRef
      }
      enumValues(includeDeprecated: true) {
        name
        description
        isDeprecated
        deprecationReason
      }
      possibleTypes {
        ...TypeRef
      }
    }

    fragment InputValue on __InputValue {
      name
      description
      type {
        ...TypeRef
      }
      defaultValue
    }

    fragment TypeRef on __Type {
      kind
      name
      ofType {
        kind
        name
        ofType {
          kind
          name
          ofType {
            kind
            name
            ofType {
              kind
              name
              ofType {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
            }
          }
        }
      }
    }
  "
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_introspection_query_contains_required_fields() {
        let query = get_introspection_query();
        assert!(query.contains("IntrospectionQuery"));
        assert!(query.contains("__schema"));
        assert!(query.contains("queryType"));
        assert!(query.contains("mutationType"));
        assert!(query.contains("subscriptionType"));
        assert!(query.contains("includeDeprecated"));
    }

    #[test]
    fn test_introspection_query_has_fragments() {
        let query = get_introspection_query();
        assert!(query.contains("fragment FullType"));
        assert!(query.contains("fragment InputValue"));
        assert!(query.contains("fragment TypeRef"));
    }
}
