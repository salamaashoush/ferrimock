//! Query language parser and executor for filtering mocks

use super::types::{FilterOperator, QueryFilter};
use crate::engine::types::MockDefinition;
use regex::Regex;
use std::borrow::Cow;
use std::sync::Arc;

/// Parse a query filter expression
///
/// Syntax: field operator value [AND|OR field operator value]*
///
/// Examples:
/// - "priority>100"
/// - "enabled=true AND scope=test-*"
/// - "match.url~=/api/users/.*"
pub fn parse_query(query: &str) -> crate::Result<Vec<QueryFilter>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let mut filters = Vec::new();

    // Split by AND/OR (simple implementation - doesn't support nested conditions)
    let parts: Vec<&str> = query
        .split(" AND ")
        .flat_map(|part| part.split(" OR "))
        .collect();

    for part in parts {
        let filter = parse_single_filter(part.trim())?;
        filters.push(filter);
    }

    Ok(filters)
}

/// Parse a single filter expression
fn parse_single_filter(expr: &str) -> crate::Result<QueryFilter> {
    // Try to match operators in order of length (longest first to avoid partial matches)
    let operators = [">=", "<=", "~=", "^=", "$=", "*=", "!=", "=", ">", "<"];

    for op_str in &operators {
        if let Some(pos) = expr.find(op_str) {
            let field = expr
                .get(..pos)
                .ok_or_else(|| crate::mp_err!("Invalid filter expression: {expr}"))?
                .trim()
                .to_string();
            let value = expr
                .get(pos + op_str.len()..)
                .ok_or_else(|| crate::mp_err!("Invalid filter expression: {expr}"))?
                .trim()
                .to_string();
            let operator = op_str.parse::<FilterOperator>()?;

            return Ok(QueryFilter {
                field,
                operator,
                value,
            });
        }
    }

    Err(crate::mp_err!("Invalid filter expression: {expr}"))
}

/// Apply query filters to mocks
pub fn apply_filters(
    mocks: Vec<Arc<MockDefinition>>,
    filters: &[QueryFilter],
) -> Vec<Arc<MockDefinition>> {
    if filters.is_empty() {
        return mocks;
    }

    mocks
        .into_iter()
        .filter(|mock| filters.iter().all(|filter| matches_filter(mock, filter)))
        .collect()
}

/// Check if a mock matches a filter
fn matches_filter(mock: &MockDefinition, filter: &QueryFilter) -> bool {
    let value = get_field_value(mock, &filter.field);

    match &filter.operator {
        FilterOperator::Equal => value == filter.value,
        FilterOperator::NotEqual => value != filter.value,
        FilterOperator::GreaterThan => {
            if let (Ok(v1), Ok(v2)) = (value.parse::<i64>(), filter.value.parse::<i64>()) {
                v1 > v2
            } else {
                false
            }
        }
        FilterOperator::LessThan => {
            if let (Ok(v1), Ok(v2)) = (value.parse::<i64>(), filter.value.parse::<i64>()) {
                v1 < v2
            } else {
                false
            }
        }
        FilterOperator::GreaterOrEqual => {
            if let (Ok(v1), Ok(v2)) = (value.parse::<i64>(), filter.value.parse::<i64>()) {
                v1 >= v2
            } else {
                false
            }
        }
        FilterOperator::LessOrEqual => {
            if let (Ok(v1), Ok(v2)) = (value.parse::<i64>(), filter.value.parse::<i64>()) {
                v1 <= v2
            } else {
                false
            }
        }
        FilterOperator::Regex => {
            if let Ok(re) = Regex::new(&filter.value) {
                re.is_match(&value)
            } else {
                false
            }
        }
        FilterOperator::StartsWith => value.starts_with(&filter.value),
        FilterOperator::EndsWith => value.ends_with(&filter.value),
        FilterOperator::Contains => value.contains(&filter.value),
    }
}

/// Get field value from mock definition (borrows when possible to avoid allocation)
fn get_field_value<'a>(mock: &'a MockDefinition, field: &str) -> Cow<'a, str> {
    match field {
        "id" => Cow::Borrowed(&mock.id),
        "scope" => mock
            .scope
            .as_deref()
            .map_or(Cow::Borrowed(""), Cow::Borrowed),
        "priority" => Cow::Owned(mock.priority.to_string()),
        "enabled" => Cow::Owned(mock.enabled.to_string()),
        _ => Cow::Borrowed(""),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::engine::types::{BodySource, RequestMatcher, ResponseGenerator};
    use axum::http::{Method, StatusCode};
    use smallvec::smallvec;

    fn create_test_mock(
        id: &str,
        priority: u32,
        enabled: bool,
        scope: Option<String>,
    ) -> MockDefinition {
        MockDefinition {
            id: id.into(),
            priority,
            enabled,
            once: false,
            scope: scope.map(Into::into),
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![],
                header_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
                query_matchers: smallvec![],
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        }
    }

    #[test]
    fn test_parse_single_filter() {
        let filter = parse_single_filter("priority>100").unwrap();
        assert_eq!(filter.field, "priority");
        assert!(matches!(filter.operator, FilterOperator::GreaterThan));
        assert_eq!(filter.value, "100");
    }

    #[test]
    fn test_parse_query_multiple_filters() {
        let filters = parse_query("priority>100 AND enabled=true").unwrap();
        assert_eq!(filters.len(), 2);
    }

    #[test]
    fn test_apply_priority_filter() {
        let mocks = vec![
            create_test_mock("mock1", 50, true, None),
            create_test_mock("mock2", 150, true, None),
            create_test_mock("mock3", 200, true, None),
        ];

        let filters = parse_query("priority>100").unwrap();
        let arc_mocks: Vec<Arc<MockDefinition>> = mocks.into_iter().map(Arc::new).collect();
        let result = apply_filters(arc_mocks, &filters);

        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|m| m.priority > 100));
    }

    #[test]
    fn test_apply_enabled_filter() {
        let mocks = vec![
            create_test_mock("mock1", 100, true, None),
            create_test_mock("mock2", 100, false, None),
            create_test_mock("mock3", 100, true, None),
        ];

        let filters = parse_query("enabled=true").unwrap();
        let arc_mocks: Vec<Arc<MockDefinition>> = mocks.into_iter().map(Arc::new).collect();
        let result = apply_filters(arc_mocks, &filters);

        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|m| m.enabled));
    }

    #[test]
    fn test_apply_scope_filter() {
        let mocks = vec![
            create_test_mock("mock1", 100, true, Some("test".to_string())),
            create_test_mock("mock2", 100, true, Some("prod".to_string())),
            create_test_mock("mock3", 100, true, Some("test-integration".to_string())),
        ];

        let filters = parse_query("scope^=test").unwrap();
        let arc_mocks: Vec<Arc<MockDefinition>> = mocks.into_iter().map(Arc::new).collect();
        let result = apply_filters(arc_mocks, &filters);

        assert_eq!(result.len(), 2);
    }
}
