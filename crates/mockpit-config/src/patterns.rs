//! URL pattern parsing utilities

use mockpit_types::UrlPattern;
use std::fmt::Write;

/// Check if pattern contains regex escape sequences
///
/// Detects backslash-escaped characters commonly used in regex patterns:
/// - Character classes: `\d`, `\D`, `\w`, `\W`, `\s`, `\S`
/// - Boundaries: `\b`, `\B`
/// - Special chars: `\.`, `\/`, `\[`, `\]`, `\(`, `\)`, `\{`, `\}`, `\+`, `\?`, `\*`
/// - Whitespace: `\n`, `\r`, `\t`
fn contains_regex_escape(pattern: &str) -> bool {
    let regex_escapes = [
        "\\d", "\\D", "\\w", "\\W", "\\s", "\\S", // Character classes
        "\\b", "\\B", // Word boundaries
        "\\.", "\\/", "\\[", "\\]", "\\(", "\\)", "\\{", "\\}", // Escaped special chars
        "\\+", "\\?", "\\*", // Escaped quantifiers
        "\\n", "\\r", "\\t", // Whitespace chars
    ];

    regex_escapes.iter().any(|esc| pattern.contains(esc))
}

/// Parse URL pattern from string with auto-detection only
///
/// Auto-detection rules (no explicit prefixes):
/// - `/api/users/:id` - Express-style params → regex with captures
/// - `/api/**/*.json` - Contains ** or * → glob pattern
/// - `^/api/v\d+/users$` - Starts with ^ or regex chars (including `\.`) → regex
/// - `/api/users` - Simple path → exact match
///
/// # Examples
///
/// - `"/api/users/:id"` → regex with named capture
/// - `"/api/**/profile"` → glob pattern
/// - `"^/api/v\\d+/"` → regex pattern
/// - `"/api/users"` → exact match
pub fn parse_url_pattern(pattern: &str) -> Result<UrlPattern, String> {
    // Auto-detection only - no explicit prefixes

    // Check for Express-style params (:param or {param})
    if pattern.contains("/:") || pattern.contains("/{") {
        let regex_pattern = convert_express_to_regex(pattern);
        return UrlPattern::regex(&regex_pattern).map_err(|e| {
            format!("Failed to convert Express-style pattern '{pattern}' to regex: {e}")
        });
    }

    // Check for glob patterns (**/ or *)
    if pattern.contains("**") || (pattern.contains('*') && !pattern.starts_with('^')) {
        return UrlPattern::glob(pattern)
            .map_err(|e| format!("Invalid glob pattern '{pattern}': {e}"));
    }

    // Check for regex patterns (starts with ^ or ends with $, or contains regex special chars)
    //
    // For `[` and `(`, only check the path portion (before `?`) because query strings
    // commonly contain literal brackets (e.g. PHP array params `?fileIDs[]=123` or
    // OData filters `?filter=(name eq 'test')`). These are NOT regex patterns.
    // If someone needs regex matching on query strings, they should use regex escapes
    // (e.g. `\?`) which are caught by `contains_regex_escape` above.
    let path_part = pattern.split('?').next().unwrap_or(pattern);

    if pattern.starts_with('^')
        || pattern.ends_with('$')
        || contains_regex_escape(pattern)
        || path_part.contains('[')
        || path_part.contains('(')
        || pattern.contains(".+")
        || pattern.contains(".*")
        || pattern.contains('|')
    {
        return UrlPattern::regex(pattern)
            .map_err(|e| format!("Invalid regex pattern '{pattern}': {e}"));
    }

    // Default to exact match for simple paths
    Ok(UrlPattern::exact(pattern))
}

/// Convert Express-style route pattern to regex with named captures
///
/// Examples:
/// - `/api/users/:id` → `^/api/users/(?P<id>[^/]+)$`
/// - `/api/users/{id}` → `^/api/users/(?P<id>[^/]+)$`
/// - `/api/users/:user_id/files/:file_id` → `^/api/users/(?P<user_id>[^/]+)/files/(?P<file_id>[^/]+)$`
/// - `/api/users/:id?` → `^/api/users(/(?P<id>[^/]+))?$` (optional param)
///
/// Supports:
/// - Named params: `:param_name` or `{param_name}` → `(?P<param_name>[^/]+)`
/// - Optional params: `:param?` → `(/(?P<param>[^/]+))?`
/// - Wildcards: `*` → `[^/]*`, `**` → `.*`
pub fn convert_express_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                // Curly brace style parameter: {param_name}
                let mut param_name = String::new();

                // Collect parameter name until closing brace
                while let Some(&next_ch) = chars.peek() {
                    if next_ch == '}' {
                        chars.next(); // consume '}'
                        break;
                    } else if next_ch.is_alphanumeric() || next_ch == '_' {
                        if let Some(c) = chars.next() {
                            param_name.push(c);
                        }
                    } else {
                        break;
                    }
                }

                if !param_name.is_empty() {
                    let _ = write!(regex, "(?P<{param_name}>[^/]+)");
                }
            }
            ':' => {
                // Colon style parameter: :param_name
                let mut param_name = String::new();
                let mut is_optional = false;

                // Collect parameter name
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_alphanumeric() || next_ch == '_' {
                        if let Some(c) = chars.next() {
                            param_name.push(c);
                        }
                    } else if next_ch == '?' {
                        // Optional parameter
                        is_optional = true;
                        chars.next(); // consume '?'
                        break;
                    } else {
                        break;
                    }
                }

                if is_optional {
                    // For optional params, we need to backtrack and include the preceding / in the optional group
                    // Remove last character from regex if it's /
                    if regex.ends_with('/') {
                        regex.pop();
                        let _ = write!(regex, "(/(?P<{param_name}>[^/]+))?");
                    } else {
                        let _ = write!(regex, "(?P<{param_name}>[^/]+)?");
                    }
                } else {
                    let _ = write!(regex, "(?P<{param_name}>[^/]+)");
                }
            }
            '*' => {
                // Check for ** (match across segments)
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                    regex.push_str(".*");
                } else {
                    // Single * (match within segment)
                    regex.push_str("[^/]*");
                }
            }
            // Escape regex special characters (note: '{' is handled separately above for param syntax)
            '.' | '+' | '(' | ')' | '[' | ']' | '}' | '^' | '$' | '|' | '\\' | '?' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => {
                regex.push(ch);
            }
        }
    }

    regex.push('$');
    regex
}

/// Check if a string is a valid HTTP method
pub fn is_valid_http_method(s: &str) -> bool {
    matches!(
        s.to_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD" | "OPTIONS" | "TRACE" | "CONNECT"
    )
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_pattern_default_exact() {
        let pattern = parse_url_pattern("/api/users").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Exact(_)));
    }

    #[test]
    fn test_parse_url_pattern_express_style() {
        let pattern = parse_url_pattern("/api/users/:id").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));

        // Test that it matches and captures
        if let UrlPattern::Regex(regex) = pattern {
            assert!(regex.is_match("/api/users/123"));
            assert!(!regex.is_match("/api/users/"));
            assert!(!regex.is_match("/api/users/123/extra"));

            let caps = regex
                .captures("/api/users/456")
                .expect("Failed to capture regex groups");
            assert_eq!(
                caps.name("id").expect("id capture should exist").as_str(),
                "456"
            );
        }
    }

    #[test]
    fn test_parse_url_pattern_express_multiple_params() {
        let pattern = parse_url_pattern("/api/users/:user_id/files/:file_id")
            .expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));

        if let UrlPattern::Regex(regex) = pattern {
            assert!(regex.is_match("/api/users/123/files/456"));

            let caps = regex
                .captures("/api/users/abc/files/xyz")
                .expect("Failed to capture regex groups");
            assert_eq!(
                caps.name("user_id")
                    .expect("user_id capture should exist")
                    .as_str(),
                "abc"
            );
            assert_eq!(
                caps.name("file_id")
                    .expect("file_id capture should exist")
                    .as_str(),
                "xyz"
            );
        }
    }

    #[test]
    fn test_parse_url_pattern_auto_detect_glob() {
        let pattern = parse_url_pattern("/api/**/*.json").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Glob(_)));
    }

    #[test]
    fn test_parse_url_pattern_auto_detect_regex() {
        // Test regex detection by special chars
        let pattern1 = parse_url_pattern("^/api/users/\\d+$").expect("Failed to parse URL pattern");
        assert!(matches!(pattern1, UrlPattern::Regex(_)));

        let pattern2 = parse_url_pattern("/api/users/[0-9]+").expect("Failed to parse URL pattern");
        assert!(matches!(pattern2, UrlPattern::Regex(_)));

        let pattern3 = parse_url_pattern("/api/.+").expect("Failed to parse URL pattern");
        assert!(matches!(pattern3, UrlPattern::Regex(_)));
    }

    #[test]
    fn test_convert_express_to_regex_basic() {
        let regex_str = convert_express_to_regex("/api/users/:id");
        assert_eq!(regex_str, "^/api/users/(?P<id>[^/]+)$");
    }

    #[test]
    fn test_convert_express_to_regex_multiple_params() {
        let regex_str = convert_express_to_regex("/api/:version/users/:id");
        assert_eq!(regex_str, "^/api/(?P<version>[^/]+)/users/(?P<id>[^/]+)$");
    }

    #[test]
    fn test_convert_express_to_regex_optional_param() {
        let regex_str = convert_express_to_regex("/api/users/:id?");
        assert_eq!(regex_str, "^/api/users(/(?P<id>[^/]+))?$");
    }

    #[test]
    fn test_parse_url_pattern_escaped_dot() {
        // Pattern with escaped dot should be detected as regex
        let pattern =
            parse_url_pattern("/api/images/avatar\\.png").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));

        if let UrlPattern::Regex(regex) = pattern {
            assert!(regex.is_match("/api/images/avatar.png"));
            assert!(!regex.is_match("/api/images/avatarXpng"));
        }
    }

    #[test]
    fn test_parse_url_pattern_word_boundary() {
        // Pattern with word boundary should be detected as regex
        let pattern = parse_url_pattern("/api/\\btest\\b").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));
    }

    #[test]
    fn test_parse_url_pattern_negated_class() {
        // Pattern with negated character class should be detected as regex
        let pattern = parse_url_pattern("/api/\\D+").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));

        if let UrlPattern::Regex(regex) = pattern {
            assert!(regex.is_match("/api/ABC"));
            assert!(!regex.is_match("/api/123"));
        }
    }

    #[test]
    fn test_parse_url_pattern_alternation() {
        // Pattern with alternation (pipe) should be detected as regex
        let pattern = parse_url_pattern("/api/(foo|bar)$").expect("Failed to parse URL pattern");
        assert!(matches!(pattern, UrlPattern::Regex(_)));

        if let UrlPattern::Regex(regex) = pattern {
            assert!(regex.is_match("/api/foo"));
            assert!(regex.is_match("/api/bar"));
            assert!(!regex.is_match("/api/baz"));
        }
    }

    #[test]
    fn test_parse_url_pattern_escaped_special_chars() {
        // Pattern with escaped special chars should be detected as regex
        let pattern1 = parse_url_pattern("/api/\\[test\\]").expect("Failed to parse");
        assert!(matches!(pattern1, UrlPattern::Regex(_)));

        let pattern2 = parse_url_pattern("/api/\\+test").expect("Failed to parse");
        assert!(matches!(pattern2, UrlPattern::Regex(_)));

        let pattern3 = parse_url_pattern("/api/test\\/path").expect("Failed to parse");
        assert!(matches!(pattern3, UrlPattern::Regex(_)));
    }

    // ===========================================================================
    // Query string bracket tests - ensure literal brackets in query params
    // are NOT misdetected as regex patterns
    // ===========================================================================

    #[test]
    fn test_parse_url_pattern_php_array_brackets_in_query() {
        // PHP-style array params: fileIDs[]=123
        let pattern =
            parse_url_pattern("/index.php?fileIDs[]=18142964699&rm=preview_get_files_metadata")
                .expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Exact(_)),
            "PHP array brackets in query string should be exact match, not regex"
        );
    }

    #[test]
    fn test_parse_url_pattern_nested_brackets_in_query() {
        // Nested query params: filter[status]=active
        let pattern = parse_url_pattern("/api/items?filter[status]=active&filter[type]=doc")
            .expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Exact(_)),
            "Nested brackets in query string should be exact match"
        );
    }

    #[test]
    fn test_parse_url_pattern_multi_value_brackets_in_query() {
        // Multi-value params: fields[]=name&fields[]=id
        let pattern = parse_url_pattern("/api/users?fields[]=name&fields[]=id&fields[]=email")
            .expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Exact(_)),
            "Multi-value brackets in query string should be exact match"
        );
    }

    #[test]
    fn test_parse_url_pattern_parentheses_in_query() {
        // OData-style filters: $filter=(name eq 'test')
        let pattern = parse_url_pattern("/api/search?$filter=(name eq 'test')&$top=10")
            .expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Exact(_)),
            "Parentheses in query string should be exact match"
        );
    }

    #[test]
    fn test_parse_url_pattern_brackets_in_path_still_regex() {
        // Brackets in the PATH portion should still be detected as regex
        let pattern = parse_url_pattern("/api/users/[0-9]+").expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Regex(_)),
            "Brackets in path should still be regex"
        );
    }

    #[test]
    fn test_parse_url_pattern_parentheses_in_path_still_regex() {
        // Parentheses in the PATH portion should still be detected as regex
        let pattern = parse_url_pattern("/api/(foo|bar)/items$").expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Regex(_)),
            "Parentheses in path should still be regex"
        );
    }

    #[test]
    fn test_parse_url_pattern_mixed_path_regex_with_query() {
        // Regex in path + literal query string (uses regex escapes so caught by contains_regex_escape)
        let pattern = parse_url_pattern("/api/users/\\d+\\?page=1").expect("Failed to parse");
        assert!(
            matches!(pattern, UrlPattern::Regex(_)),
            "Regex escape in pattern should still be detected as regex"
        );
    }

    #[test]
    fn test_parse_url_pattern_complex_recorded_url_with_query() {
        // Real recorded URLs should parse as exact matches
        let urls = [
            "/app-api/end-user-web/metadata-instances:bulk?fileIDs=18142964699&templateKey=boxSign",
            "/index.php?rm=box_gen204_batch_record",
            "/app-api/enduserapp/onboarding/experiences?context=all_files",
            "/api/v2/files/123?fields=name,size,modified_at",
        ];

        for url in &urls {
            let pattern =
                parse_url_pattern(url).unwrap_or_else(|e| panic!("Failed to parse '{url}': {e}"));
            assert!(
                matches!(pattern, UrlPattern::Exact(_)),
                "Recorded URL '{}' should be exact match, got {:?}",
                url,
                std::mem::discriminant(&pattern)
            );
        }
    }
}
