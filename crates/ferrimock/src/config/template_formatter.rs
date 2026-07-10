//! Formatter for mock body strings containing JSON mixed with Tera template expressions.
//!
//! Standard JSON parsers can't handle Tera expressions (`{{ }}`, `{% %}`, `{# #}`),
//! so this module provides a custom state-machine formatter that understands both.
//!
//! Like Prettier, the formatter is **opinionated** - it always produces correct, consistent
//! output regardless of how messy the input is. Original newlines are used only to detect
//! user intent (structural vs inline Tera blocks).

/// Format a mock body string. Handles:
/// - Pure JSON (delegates to serde_json pretty-print)
/// - JSON-with-Tera (custom state-machine formatter)
/// - Plain text with Tera (normalize delimiters only)
pub fn format_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return body.to_string();
    }

    // Try pure JSON first (no Tera syntax)
    if !has_tera_syntax(trimmed) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return serde_json::to_string_pretty(&value).unwrap_or_else(|_| body.to_string());
        }
        // Not JSON, not Tera -- return as-is
        return body.to_string();
    }

    // Has Tera syntax -- check if it looks like JSON-with-Tera
    if looks_like_json_with_tera(trimmed) {
        return format_json_with_tera(trimmed);
    }

    // Plain text with Tera -- only normalize delimiters
    normalize_tera_delimiters(trimmed)
}

/// Check if text contains Tera template syntax
fn has_tera_syntax(s: &str) -> bool {
    s.contains("{{") || s.contains("{%") || s.contains("{#")
}

/// Check if text looks like JSON structure with Tera expressions mixed in.
///
/// Distinguishes JSON-with-Tera (e.g., `{%- set x -%}{"key": "{{ val }}"}`) from
/// pure Tera expressions (e.g., `{{ fake_pdf(...) }}`). Leading `{% %}` block tags
/// and `{# #}` comments are skipped; the first remaining `{` must NOT be followed
/// by `{`, `%`, or `#` (which would indicate another Tera delimiter, not JSON).
#[allow(clippy::indexing_slicing)]
fn looks_like_json_with_tera(s: &str) -> bool {
    let trimmed = s.trim();
    // Must start with { or [
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return false;
    }
    // Must end with } or ]
    if !trimmed.ends_with('}') && !trimmed.ends_with(']') {
        return false;
    }

    // Skip leading Tera block tags ({%...%}) and comments ({#...#}) to find the
    // actual JSON start. We do NOT skip {{ expressions }} here because those are
    // values, not structural blocks that precede JSON.
    let chars: Vec<char> = trimmed.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i + 1 < len {
        if chars[i] == '{'
            && matches!(chars[i + 1], '%' | '#')
            && let Some((_, end)) = extract_tera_block(&chars, i)
        {
            i = end;
            // Skip whitespace between blocks
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        break;
    }

    if i >= len {
        return false; // Only Tera blocks, no JSON content
    }

    // After skipping leading blocks, the next char must be a JSON opener
    if chars[i] == '[' {
        return true;
    }
    if chars[i] == '{' {
        // Must not be another Tera delimiter ({{, {%, {#)
        if i + 1 < len && matches!(chars[i + 1], '{' | '%' | '#') {
            return false;
        }
        return true;
    }

    false
}

/// Extract the first keyword from a Tera block tag content.
/// Given `{% for i in range(end=3) %}`, returns `"for"`.
/// Given `{%- set x = 1 -%}`, returns `"set"`.
fn extract_tera_keyword(block: &str) -> Option<&str> {
    // Must be a block tag {% ... %}
    if !block.starts_with("{%") {
        return None;
    }
    let inner = block.strip_prefix("{%")?.strip_suffix("%}")?;
    // Strip trim markers
    let inner = inner.strip_prefix('-').unwrap_or(inner);
    let inner = inner.strip_suffix('-').unwrap_or(inner);
    let inner = inner.trim();
    // First word is the keyword
    inner.split_whitespace().next()
}

/// Check if a range of the input chars contains a newline
#[allow(clippy::indexing_slicing)]
fn has_newline_in_range(chars: &[char], start: usize, end: usize) -> bool {
    let end = end.min(chars.len());
    for c in &chars[start..end] {
        if *c == '\n' {
            return true;
        }
    }
    false
}

/// Check if output already ends with newline + proper indent
fn output_ends_with_indent(output: &str, depth: usize, indent_str: &str) -> bool {
    let mut expected = String::from('\n');
    for _ in 0..depth {
        expected.push_str(indent_str);
    }
    output.ends_with(&expected)
}

/// Ensure output ends with newline + indent at the given depth.
/// If output is empty, just push indent (no leading newline).
fn ensure_newline_indent(output: &mut String, depth: usize, indent_str: &str) {
    if output.is_empty() {
        push_indent(output, depth, indent_str);
        return;
    }
    if !output_ends_with_indent(output, depth, indent_str) {
        // Trim any trailing whitespace-only content after last newline
        // to avoid double-indenting
        if let Some(last_nl) = output.rfind('\n') {
            let after_nl = output.get(last_nl + 1..).unwrap_or("");
            if after_nl.chars().all(|c| c == ' ' || c == '\t') {
                output.truncate(last_nl + 1);
            }
        }
        if !output.ends_with('\n') {
            output.push('\n');
        }
        push_indent(output, depth, indent_str);
    }
}

/// Check if the content between an `{% if %}` and its matching `{% endif %}` contains
/// newlines in the original input. Used to distinguish truly inline if-blocks
/// (e.g., `{% if not loop.last %},{% endif %}`) from if-blocks that wrap multi-line
/// content and should be treated as structural.
#[allow(clippy::indexing_slicing)]
fn if_content_has_newline(chars: &[char], start: usize) -> bool {
    let mut depth: u32 = 1;
    let len = chars.len();
    let mut i = start;
    while i < len {
        if i + 1 < len
            && chars[i] == '{'
            && chars[i + 1] == '%'
            && let Some((block, end)) = extract_tera_block(chars, i)
        {
            if let Some(kw) = extract_tera_keyword(&block) {
                match kw {
                    "if" => depth += 1,
                    "endif" => {
                        depth -= 1;
                        if depth == 0 {
                            return has_newline_in_range(chars, start, i);
                        }
                    }
                    _ => {}
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    false // No matching endif found
}

/// Keywords that are always structural (get their own line)
fn is_always_structural(keyword: &str) -> bool {
    matches!(
        keyword,
        "for" | "endfor" | "set" | "block" | "endblock" | "macro" | "endmacro" | "else" | "elif"
    )
}

/// Format JSON content that contains Tera template expressions.
///
/// Uses a character-level state machine that tracks:
/// - JSON structural depth for indentation
/// - Tera block tag classification (structural vs inline)
/// - Inline Tera depth for context-dependent if/endif
/// - Tera block depth for indenting nested control structures
/// - JSON string literals (not reformatted internally)
#[allow(clippy::indexing_slicing)]
fn format_json_with_tera(input: &str) -> String {
    let mut output = String::with_capacity(input.len() * 2);
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut depth: usize = 0;
    let indent_str = "  ";

    // State tracking
    let mut in_string = false;
    let mut inline_tera_depth: u32 = 0; // tracks inline {% if %}...{% endif %} nesting
    let mut tera_block_depth: usize = 0; // tracks structural Tera block nesting for indentation
    let mut last_token_end: usize = 0; // end position of last emitted token in input
    let mut need_newline_after_structural = false; // set after structural Tera block, cleared by next JSON token

    while i < len {
        // Handle Tera blocks outside of strings
        if in_string {
            // Inside a JSON string -- copy verbatim (including Tera expressions within strings)
            match chars[i] {
                '\\' => {
                    // Escaped character -- copy both
                    output.push('\\');
                    i += 1;
                    if i < len {
                        output.push(chars[i]);
                        i += 1;
                    }
                }
                '"' => {
                    // End of string
                    output.push('"');
                    in_string = false;
                    last_token_end = i + 1;
                    i += 1;
                }
                _ => {
                    output.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            // Check for Tera delimiters: {{ }}, {% %}, {# #}
            if i + 1 < len && chars[i] == '{' && matches!(chars[i + 1], '{' | '%' | '#') {
                let tera_block = extract_tera_block(&chars, i);
                if let Some((block_content, end_pos)) = tera_block {
                    let normalized = normalize_tera_block(&block_content);

                    // Classify: is this a block tag {% ... %} ?
                    if let Some(keyword) = extract_tera_keyword(&block_content) {
                        if is_always_structural(keyword) {
                            // Always structural -- gets its own line
                            if inline_tera_depth == 0 {
                                // Decrease depth BEFORE outputting closing tags so they align with opening tags
                                if keyword == "endfor" {
                                    tera_block_depth = tera_block_depth.saturating_sub(1);
                                }

                                // else/elif stay at same level as opening if, others use current depth
                                let indent_depth = if matches!(keyword, "else" | "elif") {
                                    depth + tera_block_depth.saturating_sub(1)
                                } else {
                                    depth + tera_block_depth
                                };
                                ensure_newline_indent(&mut output, indent_depth, indent_str);
                            }
                            output.push_str(&normalized);

                            // Update Tera block depth after outputting opening tags
                            if keyword == "for" {
                                tera_block_depth += 1;
                            }

                            need_newline_after_structural = inline_tera_depth == 0;
                            last_token_end = end_pos;
                            i = end_pos;
                            // Skip whitespace after structural block
                            while i < len && chars[i].is_whitespace() {
                                i += 1;
                            }
                            continue;
                        } else if keyword == "if" {
                            // Context-dependent: check if original had newline before this block
                            let had_newline = has_newline_in_range(&chars, last_token_end, i);
                            let at_line_start = output_ends_with_indent(
                                &output,
                                depth + tera_block_depth,
                                indent_str,
                            ) || output.ends_with('\n')
                                || output.is_empty();

                            if had_newline || (inline_tera_depth == 0 && at_line_start) {
                                // Structural if
                                if inline_tera_depth == 0 {
                                    ensure_newline_indent(
                                        &mut output,
                                        depth + tera_block_depth,
                                        indent_str,
                                    );
                                }
                                output.push_str(&normalized);
                                tera_block_depth += 1; // Increase depth after outputting if
                                need_newline_after_structural = inline_tera_depth == 0;
                                last_token_end = end_pos;
                                i = end_pos;
                                // Skip whitespace after structural block
                                while i < len && chars[i].is_whitespace() {
                                    i += 1;
                                }
                                continue;
                            }

                            // Before going inline, check if content between if/endif has
                            // newlines (multi-line body should stay structural, not collapse)
                            if if_content_has_newline(&chars, end_pos) {
                                if inline_tera_depth == 0 {
                                    ensure_newline_indent(
                                        &mut output,
                                        depth + tera_block_depth,
                                        indent_str,
                                    );
                                }
                                output.push_str(&normalized);
                                tera_block_depth += 1; // Increase depth after outputting if
                                need_newline_after_structural = inline_tera_depth == 0;
                                last_token_end = end_pos;
                                i = end_pos;
                                while i < len && chars[i].is_whitespace() {
                                    i += 1;
                                }
                                continue;
                            }

                            // Truly inline if (e.g., {% if not loop.last %},{% endif %})
                            inline_tera_depth += 1;
                            output.push_str(&normalized);
                            last_token_end = end_pos;
                            i = end_pos;
                            continue;
                        } else if keyword == "endif" {
                            if inline_tera_depth > 0 {
                                // Inline endif
                                inline_tera_depth -= 1;
                                output.push_str(&normalized);
                                last_token_end = end_pos;
                                i = end_pos;
                                continue;
                            }

                            // Structural endif - decrease depth BEFORE outputting so endif aligns with if
                            tera_block_depth = tera_block_depth.saturating_sub(1);
                            ensure_newline_indent(
                                &mut output,
                                depth + tera_block_depth,
                                indent_str,
                            );
                            output.push_str(&normalized);
                            need_newline_after_structural = true;
                            last_token_end = end_pos;
                            i = end_pos;
                            // Skip whitespace after structural block
                            while i < len && chars[i].is_whitespace() {
                                i += 1;
                            }
                            continue;
                        }
                    }

                    // Expression {{ ... }} or comment {# ... #} -- emit inline
                    // But if we just had a structural block, add newline first
                    if need_newline_after_structural && inline_tera_depth == 0 {
                        ensure_newline_indent(&mut output, depth + tera_block_depth, indent_str);
                        need_newline_after_structural = false;
                    }
                    output.push_str(&normalized);
                    last_token_end = end_pos;
                    i = end_pos;
                    continue;
                }
            }

            // Skip whitespace between tokens
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }

            // In inline mode, emit JSON chars without formatting
            if inline_tera_depth > 0 {
                output.push(chars[i]);
                last_token_end = i + 1;
                i += 1;
                continue;
            }

            // If we just emitted a structural Tera block, ensure a newline before JSON content
            if need_newline_after_structural {
                match chars[i] {
                    // Closers handle their own newlines
                    '}' | ']' => {}
                    _ => {
                        // Only add newline if we actually have prior content
                        if !output.is_empty() {
                            ensure_newline_indent(
                                &mut output,
                                depth + tera_block_depth,
                                indent_str,
                            );
                        }
                    }
                }
                need_newline_after_structural = false;
            }

            match chars[i] {
                '{' | '[' => {
                    output.push(chars[i]);
                    depth += 1;

                    // Check if the object/array is empty (next non-ws char is } or ])
                    // Use peek_non_whitespace (NOT _non_tera) so Tera blocks inside prevent collapse
                    let closing = if chars[i] == '{' { '}' } else { ']' };
                    if let Some(next) = peek_non_whitespace(&chars, i + 1)
                        && next == closing
                    {
                        // Empty object/array -- skip to closing brace
                        output.push(closing);
                        let mut j = i + 1;
                        while j < len && chars[j] != closing {
                            j += 1;
                        }
                        i = j + 1;
                        depth -= 1;
                        last_token_end = i;
                        continue;
                    }

                    output.push('\n');
                    push_indent(&mut output, depth + tera_block_depth, indent_str);
                    last_token_end = i + 1;
                    i += 1;
                }
                '}' | ']' => {
                    depth = depth.saturating_sub(1);
                    output.push('\n');
                    push_indent(&mut output, depth + tera_block_depth, indent_str);
                    output.push(chars[i]);
                    last_token_end = i + 1;
                    i += 1;
                }
                ',' => {
                    output.push(',');
                    output.push('\n');
                    push_indent(&mut output, depth + tera_block_depth, indent_str);
                    last_token_end = i + 1;
                    i += 1;
                }
                ':' => {
                    output.push(':');
                    output.push(' ');
                    last_token_end = i + 1;
                    i += 1;
                }
                '"' => {
                    // Start of JSON string
                    in_string = true;
                    output.push('"');
                    last_token_end = i + 1;
                    i += 1;
                }
                _ => {
                    // JSON literals: numbers, true, false, null
                    output.push(chars[i]);
                    last_token_end = i + 1;
                    i += 1;
                }
            }
        }
    }

    output
}

/// Extract a complete Tera block starting at position `start`.
/// Returns the block content (including delimiters) and the position after the block.
#[allow(clippy::indexing_slicing)]
fn extract_tera_block(chars: &[char], start: usize) -> Option<(String, usize)> {
    let len = chars.len();
    if start + 1 >= len {
        return None;
    }

    let second = chars[start + 1];
    let (closing_first, closing_second) = match second {
        '{' => ('}', '}'),
        '%' => ('%', '}'),
        '#' => ('#', '}'),
        _ => return None,
    };

    let mut i = start + 2;
    let mut block = String::new();
    block.push(chars[start]);
    block.push(second);

    while i < len {
        if i + 1 < len && chars[i] == closing_first && chars[i + 1] == closing_second {
            block.push(closing_first);
            block.push(closing_second);
            return Some((block, i + 2));
        }
        block.push(chars[i]);
        i += 1;
    }

    // Unclosed block -- return what we have
    None
}

/// Normalize whitespace inside a Tera block.
/// `{{name}}` -> `{{ name }}`, `{%if x%}` -> `{% if x %}`
/// Preserves whitespace-trim markers: `{%-` / `-%}`, `{{-` / `-}}`
#[allow(clippy::indexing_slicing)]
fn normalize_tera_block(block: &str) -> String {
    let chars: Vec<char> = block.chars().collect();
    let len = chars.len();
    if len < 4 {
        return block.to_string();
    }

    // Determine opening and closing delimiters
    let (open, close) = if block.starts_with("{{") {
        ("{{", "}}")
    } else if block.starts_with("{%") {
        ("{%", "%}")
    } else if block.starts_with("{#") {
        ("{#", "#}")
    } else {
        return block.to_string();
    };

    // Extract the inner content (between delimiters)
    let inner_start = 2;
    let inner_end = len - 2;
    if inner_start >= inner_end {
        return block.to_string();
    }

    let inner: String = chars
        .get(inner_start..inner_end)
        .map(|slice| slice.iter().collect())
        .unwrap_or_default();

    // Check for trim markers
    let (open_trim, content_start) = if let Some(stripped) = inner.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", inner.as_str())
    };

    let (close_trim, content_end) = if let Some(stripped) = content_start.strip_suffix('-') {
        ("-", stripped)
    } else {
        ("", content_start)
    };

    let trimmed_content = content_end.trim();

    format!("{open}{open_trim} {trimmed_content} {close_trim}{close}")
}

/// Peek ahead to find the next non-whitespace character (does not skip Tera blocks).
/// Used for empty container detection where `[{% for %}]` should NOT be treated as `[]`.
#[allow(clippy::indexing_slicing)]
fn peek_non_whitespace(chars: &[char], start: usize) -> Option<char> {
    let len = chars.len();
    let mut i = start;
    while i < len {
        if !chars[i].is_whitespace() {
            return Some(chars[i]);
        }
        i += 1;
    }
    None
}

/// Normalize Tera delimiters in plain (non-JSON) text.
/// Only touches the delimiter spacing, leaves everything else alone.
#[allow(clippy::indexing_slicing)]
fn normalize_tera_delimiters(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len
            && chars[i] == '{'
            && matches!(chars[i + 1], '{' | '%' | '#')
            && let Some((block, end)) = extract_tera_block(&chars, i)
        {
            output.push_str(&normalize_tera_block(&block));
            i = end;
            continue;
        }
        output.push(chars[i]);
        i += 1;
    }

    output
}

fn push_indent(output: &mut String, depth: usize, indent_str: &str) {
    for _ in 0..depth {
        output.push_str(indent_str);
    }
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

    // Helper: assert formatting produces exact output AND is idempotent
    fn assert_formats_to(input: &str, expected: &str) {
        let result = format_body(input);
        assert_eq!(
            result, expected,
            "\n--- INPUT ---\n{input}\n--- EXPECTED ---\n{expected}\n--- GOT ---\n{result}\n"
        );
        let second = format_body(&result);
        assert_eq!(
            result, second,
            "NOT IDEMPOTENT\n--- FIRST ---\n{result}\n--- SECOND ---\n{second}\n"
        );
    }

    // ==========================================================================
    // Pure JSON (no Tera) -- delegates to serde_json pretty-print
    // ==========================================================================

    #[test]
    fn test_pure_json_flat_object() {
        assert_formats_to(
            r#"{"id":"123","name":"test"}"#,
            "{\n  \"id\": \"123\",\n  \"name\": \"test\"\n}",
        );
    }

    #[test]
    fn test_pure_json_nested_objects() {
        assert_formats_to(
            r#"{"user":{"id":"123","address":{"city":"NYC"}}}"#,
            "\
{\
\n  \"user\": {\
\n    \"id\": \"123\",\
\n    \"address\": {\
\n      \"city\": \"NYC\"\
\n    }\
\n  }\
\n}",
        );
    }

    #[test]
    fn test_pure_json_array_of_objects() {
        assert_formats_to(
            r#"[{"id":1},{"id":2}]"#,
            "\
[\
\n  {\
\n    \"id\": 1\
\n  },\
\n  {\
\n    \"id\": 2\
\n  }\
\n]",
        );
    }

    #[test]
    fn test_pure_json_booleans_null_numbers() {
        assert_formats_to(
            r#"{"active":true,"deleted":false,"value":null,"count":42,"price":19.99}"#,
            "\
{\
\n  \"active\": true,\
\n  \"deleted\": false,\
\n  \"value\": null,\
\n  \"count\": 42,\
\n  \"price\": 19.99\
\n}",
        );
    }

    #[test]
    fn test_pure_json_string_escapes_preserved() {
        assert_formats_to(
            r#"{"msg":"line1\nline2","path":"C:\\Users"}"#,
            "{\n  \"msg\": \"line1\\nline2\",\n  \"path\": \"C:\\\\Users\"\n}",
        );
    }

    #[test]
    fn test_pure_json_empty_object() {
        assert_formats_to("{}", "{}");
    }

    #[test]
    fn test_pure_json_empty_array() {
        assert_formats_to("[]", "[]");
    }

    #[test]
    fn test_pure_json_already_formatted() {
        let input = "{\n  \"id\": \"123\"\n}";
        assert_formats_to(input, input);
    }

    // ==========================================================================
    // Non-JSON, non-Tera -- returned as-is
    // ==========================================================================

    #[test]
    fn test_plain_text_unchanged() {
        assert_formats_to("Just plain text", "Just plain text");
    }

    #[test]
    fn test_empty_body() {
        assert_eq!(format_body(""), "");
        assert_eq!(format_body("   "), "   ");
    }

    // ==========================================================================
    // Plain text with Tera -- normalize delimiters only
    // ==========================================================================

    #[test]
    fn test_plain_text_tera_normalizes_delimiters() {
        assert_formats_to(
            "Hello {{name}}, welcome to {%if premium%}premium{%endif%}!",
            "Hello {{ name }}, welcome to {% if premium %}premium{% endif %}!",
        );
    }

    #[test]
    fn test_plain_text_tera_with_trim_markers() {
        assert_formats_to("Hello {{-name-}}, welcome!", "Hello {{- name -}}, welcome!");
    }

    // ==========================================================================
    // JSON-with-Tera: basic expressions {{ }}
    // ==========================================================================

    #[test]
    fn test_json_with_tera_expressions() {
        assert_formats_to(
            r#"{"id":"{{ uuid() }}","name":"{{ fake_name() }}"}"#,
            "\
{\
\n  \"id\": \"{{ uuid() }}\",\
\n  \"name\": \"{{ fake_name() }}\"\
\n}",
        );
    }

    #[test]
    fn test_json_with_tera_expression_as_value() {
        // Tera expression used as a bare JSON value (not in a string)
        assert_formats_to(
            r#"{"count":{{ total }},"active":{{ is_active }}}"#,
            "\
{\
\n  \"count\": {{ total }},\
\n  \"active\": {{ is_active }}\
\n}",
        );
    }

    #[test]
    fn test_json_strings_not_normalized() {
        // Tera expressions INSIDE JSON strings are NOT normalized (strings are verbatim)
        assert_formats_to(
            r#"{"name":"{{name}}","email":"{{  email  }}"}"#,
            "\
{\
\n  \"name\": \"{{name}}\",\
\n  \"email\": \"{{  email  }}\"\
\n}",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: structural {% set %} blocks
    // ==========================================================================

    #[test]
    fn test_single_set_before_json() {
        assert_formats_to(
            r#"{%- set x = 1 -%}{"value": {{ x }}}"#,
            "\
{%- set x = 1 -%}\
\n{\
\n  \"value\": {{ x }}\
\n}",
        );
    }

    #[test]
    fn test_multiple_set_blocks_each_on_own_line() {
        assert_formats_to(
            r#"{%- set a = 1 -%}{%- set b = 2 -%}{%- set c = 3 -%}{"a": {{ a }}, "b": {{ b }}, "c": {{ c }}}"#,
            "\
{%- set a = 1 -%}\
\n{%- set b = 2 -%}\
\n{%- set c = 3 -%}\
\n{\
\n  \"a\": {{ a }},\
\n  \"b\": {{ b }},\
\n  \"c\": {{ c }}\
\n}",
        );
    }

    #[test]
    fn test_set_blocks_with_whitespace_between() {
        // Even with lots of whitespace, output is the same
        assert_formats_to(
            "{%- set x = 1 -%}   \n\n  {%- set y = 2 -%}  \n  {\"x\": {{ x }}, \"y\": {{ y }}}",
            "\
{%- set x = 1 -%}\
\n{%- set y = 2 -%}\
\n{\
\n  \"x\": {{ x }},\
\n  \"y\": {{ y }}\
\n}",
        );
    }

    #[test]
    fn test_set_normalizes_delimiters() {
        assert_formats_to(
            r#"{%-set x=1-%}{"value": {{ x }}}"#,
            "\
{%- set x=1 -%}\
\n{\
\n  \"value\": {{ x }}\
\n}",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: structural {% for %} / {% endfor %}
    // ==========================================================================

    #[test]
    fn test_for_loop_basic() {
        assert_formats_to(
            r#"{"items":[{% for i in range(end=3) %}{{ i }}{% endfor %}]}"#,
            "\
{\
\n  \"items\": [\
\n    {% for i in range(end=3) %}\
\n      {{ i }}\
\n    {% endfor %}\
\n  ]\
\n}",
        );
    }

    #[test]
    fn test_for_loop_with_objects() {
        assert_formats_to(
            r#"{"items":[{% for i in range(end=3) %}{"id":{{ i }}}{% endfor %}]}"#,
            "\
{\
\n  \"items\": [\
\n    {% for i in range(end=3) %}\
\n      {\
\n        \"id\": {{ i }}\
\n      }\
\n    {% endfor %}\
\n  ]\
\n}",
        );
    }

    #[test]
    fn test_for_loop_with_inline_comma() {
        // The classic conditional comma pattern: }{% if not loop.last %},{% endif %}
        assert_formats_to(
            r#"{"items":[{% for i in range(end=3) %}{"id":{{ i }}}{% if not loop.last %},{% endif %}{% endfor %}]}"#,
            "\
{\
\n  \"items\": [\
\n    {% for i in range(end=3) %}\
\n      {\
\n        \"id\": {{ i }}\
\n      }{% if not loop.last %},{% endif %}\
\n    {% endfor %}\
\n  ]\
\n}",
        );
    }

    #[test]
    fn test_for_loop_multifield_objects_with_comma() {
        assert_formats_to(
            r#"[{% for i in range(end=2) %}{"id":{{ i }},"name":"{{ fake_name() }}"}{% if not loop.last %},{% endif %}{% endfor %}]"#,
            "\
[\
\n  {% for i in range(end=2) %}\
\n    {\
\n      \"id\": {{ i }},\
\n      \"name\": \"{{ fake_name() }}\"\
\n    }{% if not loop.last %},{% endif %}\
\n  {% endfor %}\
\n]",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: structural {% if %} / {% else %} / {% endif %}
    // ==========================================================================

    #[test]
    fn test_structural_if_with_newlines_in_original() {
        assert_formats_to(
            "{\n  {% if condition %}\n  \"key\": \"value\"\n  {% endif %}\n}",
            "\
{\
\n  {% if condition %}\
\n    \"key\": \"value\"\
\n  {% endif %}\
\n}",
        );
    }

    #[test]
    fn test_structural_if_else_endif() {
        assert_formats_to(
            "{% if active %}\n{\"status\": \"ok\"}\n{% else %}\n{\"status\": \"inactive\"}\n{% endif %}",
            "\
{% if active %}\
\n  {\
\n    \"status\": \"ok\"\
\n  }\
\n{% else %}\
\n  {\
\n    \"status\": \"inactive\"\
\n  }\
\n{% endif %}",
        );
    }

    #[test]
    fn test_inline_if_no_newline_stays_inline() {
        // When if/endif have no newlines between them and surrounding tokens, they stay inline
        assert_formats_to(
            r#"{"value":{% if x %}"yes"{% endif %}}"#,
            "\
{\
\n  \"value\": {% if x %}\"yes\"{% endif %}\
\n}",
        );
    }

    #[test]
    fn test_inline_if_else_endif() {
        assert_formats_to(
            r#"{"next":{% if has_more %}"{{ cursor }}"{% else %}null{% endif %}}"#,
            "\
{\
\n  \"next\": {% if has_more %}\"{{ cursor }}\"{% else %}null{% endif %}\
\n}",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: comments {# #}
    // ==========================================================================

    #[test]
    fn test_comment_before_json() {
        assert_formats_to(
            r#"{# API response #}{"id":"123"}"#,
            "\
{# API response #}{\
\n  \"id\": \"123\"\
\n}",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: delimiter normalization at structural level
    // ==========================================================================

    #[test]
    fn test_structural_blocks_normalized() {
        assert_formats_to(
            r#"{%if active%}{"status":"ok"}{%endif%}"#,
            "\
{% if active %}\
\n  {\
\n    \"status\": \"ok\"\
\n  }\
\n{% endif %}",
        );
    }

    #[test]
    fn test_trim_markers_preserved() {
        assert_formats_to(
            r#"{%-if active-%}{"status":"ok"}{%-endif-%}"#,
            "\
{%- if active -%}\
\n  {\
\n    \"status\": \"ok\"\
\n  }\
\n{%- endif -%}",
        );
    }

    // ==========================================================================
    // Full pagination pattern (real-world mock)
    // ==========================================================================

    #[test]
    fn test_pagination_pattern_exact_output() {
        let input = r#"{%- set page = query.page | default(value="1") | int -%}{%- set limit = query.limit | default(value="10") | int -%}{%- set total = 100 -%}{"page": {{ page }}, "limit": {{ limit }}, "total": {{ total }}, "items": [{% for i in range(start=0, end=limit) %}{"id": "{{ fake_uuid() }}", "name": "{{ fake_sentence(words=3) | title }}"}{% if not loop.last %},{% endif %}{% endfor %}]}"#;
        assert_formats_to(
            input,
            "\
{%- set page = query.page | default(value=\"1\") | int -%}\
\n{%- set limit = query.limit | default(value=\"10\") | int -%}\
\n{%- set total = 100 -%}\
\n{\
\n  \"page\": {{ page }},\
\n  \"limit\": {{ limit }},\
\n  \"total\": {{ total }},\
\n  \"items\": [\
\n    {% for i in range(start=0, end=limit) %}\
\n      {\
\n        \"id\": \"{{ fake_uuid() }}\",\
\n        \"name\": \"{{ fake_sentence(words=3) | title }}\"\
\n      }{% if not loop.last %},{% endif %}\
\n    {% endfor %}\
\n  ]\
\n}",
        );
    }

    // ==========================================================================
    // Offset pagination pattern (set blocks + for + nested if + conditional URLs)
    // ==========================================================================

    #[test]
    fn test_offset_pagination_pattern() {
        let input = r#"{%- set limit = query.limit | default(value="10") | int -%}{%- set offset = query.offset | default(value="0") | int -%}{%- set total = 100 -%}{"total": {{ total }}, "limit": {{ limit }}, "offset": {{ offset }}, "has_next": {{ offset + limit < total }}, "next_url": {% if offset + limit < total %}"{{ fake_api_url() }}/users?limit={{ limit }}&offset={{ offset + limit }}"{% else %}null{% endif %}, "data": [{% for i in range(start=0, end=limit) %}{% if offset + i < total %}{"id": {{ offset + i + 1 }}, "name": "{{ fake_name() }}"}{% if not loop.last %},{% endif %}{% endif %}{% endfor %}]}"#;

        let result = format_body(input);

        // Verify structural properties
        let lines: Vec<&str> = result.lines().collect();

        // All 3 set blocks on their own lines
        let set_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.trim().starts_with("{%") && l.contains(" set "))
            .copied()
            .collect();
        assert_eq!(
            set_lines.len(),
            3,
            "Should have 3 set lines. Got:\n{result}"
        );

        // JSON { on its own line after the set blocks
        let json_start_idx = lines
            .iter()
            .position(|l| l.trim() == "{")
            .expect("Should have JSON { on its own line");
        assert_eq!(
            lines[json_start_idx].trim(),
            "{",
            "JSON opener should be alone on its line"
        );

        // for on its own line
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% for")),
            "for should start its own line. Got:\n{result}"
        );

        // endfor on its own line
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% endfor")),
            "endfor should start its own line. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(
            result, second,
            "NOT IDEMPOTENT\n--- FIRST ---\n{result}\n--- SECOND ---\n{second}\n"
        );
    }

    // ==========================================================================
    // Messy / badly-formatted input -- Prettier-style normalization
    // ==========================================================================

    #[test]
    fn test_messy_whitespace_normalizes() {
        assert_formats_to(
            "{%- set x = 1 -%}   {%- set y = 2 -%}   {  \"x\":   {{ x }} ,  \"y\":  {{ y }}  }",
            "\
{%- set x = 1 -%}\
\n{%- set y = 2 -%}\
\n{\
\n  \"x\": {{ x }},\
\n  \"y\": {{ y }}\
\n}",
        );
    }

    #[test]
    fn test_cramped_json_with_tera_normalizes() {
        assert_formats_to(
            r#"{"a":"{{ x }}","b":"{{ y }}","c":{{ z }}}"#,
            "\
{\
\n  \"a\": \"{{ x }}\",\
\n  \"b\": \"{{ y }}\",\
\n  \"c\": {{ z }}\
\n}",
        );
    }

    #[test]
    fn test_already_formatted_unchanged() {
        let formatted = "\
{%- set x = 1 -%}\
\n{\
\n  \"value\": {{ x }}\
\n}";
        assert_formats_to(formatted, formatted);
    }

    // ==========================================================================
    // Empty containers in JSON-with-Tera
    // ==========================================================================

    #[test]
    fn test_empty_object_in_tera_json() {
        assert_formats_to(
            r#"{"data":{},"list":[]}"#,
            "\
{\
\n  \"data\": {},\
\n  \"list\": []\
\n}",
        );
    }

    // ==========================================================================
    // Nested objects/arrays with Tera
    // ==========================================================================

    #[test]
    fn test_deeply_nested_with_tera() {
        assert_formats_to(
            r#"{"user":{"id":"{{ uuid() }}","profile":{"name":"{{ fake_name() }}","address":{"city":"{{ fake_city() }}"}}}}"#,
            "\
{\
\n  \"user\": {\
\n    \"id\": \"{{ uuid() }}\",\
\n    \"profile\": {\
\n      \"name\": \"{{ fake_name() }}\",\
\n      \"address\": {\
\n        \"city\": \"{{ fake_city() }}\"\
\n      }\
\n    }\
\n  }\
\n}",
        );
    }

    #[test]
    fn test_array_of_objects_with_tera() {
        assert_formats_to(
            r#"{"tags":[{"id":"{{ uuid() }}","name":"{{ fake_word() }}"},{"id":"{{ uuid() }}","name":"{{ fake_word() }}"}]}"#,
            "\
{\
\n  \"tags\": [\
\n    {\
\n      \"id\": \"{{ uuid() }}\",\
\n      \"name\": \"{{ fake_word() }}\"\
\n    },\
\n    {\
\n      \"id\": \"{{ uuid() }}\",\
\n      \"name\": \"{{ fake_word() }}\"\
\n    }\
\n  ]\
\n}",
        );
    }

    // ==========================================================================
    // JSON-with-Tera: {% if %} inline conditional value pattern
    // ==========================================================================

    #[test]
    fn test_inline_conditional_null_vs_string() {
        // Pattern: "next_marker": {% if has_more %}"cursor_abc"{% else %}null{% endif %}
        assert_formats_to(
            r#"{"next_marker":{% if has_more %}"cursor_abc"{% else %}null{% endif %},"data":[]}"#,
            "\
{\
\n  \"next_marker\": {% if has_more %}\"cursor_abc\"{% else %}null{% endif %},\
\n  \"data\": []\
\n}",
        );
    }

    // ==========================================================================
    // File generation bodies (fake_pdf, fake_png) -- NOT JSON
    // ==========================================================================

    #[test]
    fn test_fake_pdf_not_treated_as_json() {
        let input = r#"{{ fake_pdf(text="Invoice #12345\nTotal: $1,234.56") }}"#;
        assert_formats_to(input, input);
    }

    #[test]
    fn test_fake_pdf_with_real_newlines() {
        let input = "{{ fake_pdf(text=\"Invoice #12345\nTotal: $1,234.56\nDate: 2024-01-15\") }}";
        assert_formats_to(input, input);
    }

    #[test]
    fn test_fake_png_not_treated_as_json() {
        let input = "{{ fake_png(width=800, height=600, color=\"#4CAF50\") }}";
        assert_formats_to(input, input);
    }

    #[test]
    fn test_set_then_fake_pdf_not_json() {
        // {%- set -%}{{ expr }} should NOT be treated as JSON
        let input = "{%- set x = 1 -%}{{ fake_name() }}";
        let result = format_body(input);
        // This is plain-text-with-Tera (normalize delimiters only)
        assert!(
            !result.contains('\n') || result.lines().count() <= 2,
            "set+expression should not be JSON-formatted. Got:\n{result}"
        );
    }

    // ==========================================================================
    // Tera block normalization (unit tests for normalize_tera_block)
    // ==========================================================================

    #[test]
    fn test_normalize_expression_spacing() {
        assert_eq!(normalize_tera_block("{{name}}"), "{{ name }}");
        assert_eq!(normalize_tera_block("{{  name  }}"), "{{ name }}");
        assert_eq!(normalize_tera_block("{{ name }}"), "{{ name }}");
    }

    #[test]
    fn test_normalize_block_tag_spacing() {
        assert_eq!(normalize_tera_block("{%if x%}"), "{% if x %}");
        assert_eq!(normalize_tera_block("{%  if x  %}"), "{% if x %}");
        assert_eq!(normalize_tera_block("{% if x %}"), "{% if x %}");
    }

    #[test]
    fn test_normalize_comment_spacing() {
        assert_eq!(normalize_tera_block("{#comment#}"), "{# comment #}");
        assert_eq!(normalize_tera_block("{# comment #}"), "{# comment #}");
    }

    #[test]
    fn test_normalize_trim_markers() {
        assert_eq!(normalize_tera_block("{{-name-}}"), "{{- name -}}");
        assert_eq!(normalize_tera_block("{%-if x-%}"), "{%- if x -%}");
        assert_eq!(normalize_tera_block("{%- if x -%}"), "{%- if x -%}");
    }

    #[test]
    fn test_normalize_complex_expression() {
        assert_eq!(
            normalize_tera_block("{{fake_name()|upper}}"),
            "{{ fake_name()|upper }}"
        );
    }

    // ==========================================================================
    // extract_tera_keyword (unit tests)
    // ==========================================================================

    #[test]
    fn test_extract_keyword_all_types() {
        assert_eq!(
            extract_tera_keyword("{% for i in range(end=3) %}"),
            Some("for")
        );
        assert_eq!(extract_tera_keyword("{% endfor %}"), Some("endfor"));
        assert_eq!(extract_tera_keyword("{%- set x = 1 -%}"), Some("set"));
        assert_eq!(extract_tera_keyword("{% if condition %}"), Some("if"));
        assert_eq!(extract_tera_keyword("{% elif other %}"), Some("elif"));
        assert_eq!(extract_tera_keyword("{% else %}"), Some("else"));
        assert_eq!(extract_tera_keyword("{% endif %}"), Some("endif"));
        assert_eq!(extract_tera_keyword("{% block content %}"), Some("block"));
        assert_eq!(extract_tera_keyword("{% endblock %}"), Some("endblock"));
        assert_eq!(extract_tera_keyword("{% macro render() %}"), Some("macro"));
        assert_eq!(extract_tera_keyword("{% endmacro %}"), Some("endmacro"));
        // Not block tags
        assert_eq!(extract_tera_keyword("{{ expression }}"), None);
        assert_eq!(extract_tera_keyword("{# comment #}"), None);
    }

    // ==========================================================================
    // looks_like_json_with_tera classification (unit tests)
    // ==========================================================================

    #[test]
    fn test_classification_json_object() {
        assert!(looks_like_json_with_tera("{\"key\": \"value\"}"));
    }

    #[test]
    fn test_classification_json_array() {
        assert!(looks_like_json_with_tera("[1, 2, 3]"));
    }

    #[test]
    fn test_classification_tera_blocks_then_json() {
        assert!(looks_like_json_with_tera(
            "{%- set x = 1 -%}{\"key\": \"value\"}"
        ));
    }

    #[test]
    fn test_classification_comment_then_json() {
        assert!(looks_like_json_with_tera("{# comment #}{\"key\": \"val\"}"));
    }

    #[test]
    fn test_classification_multiple_blocks_then_json() {
        assert!(looks_like_json_with_tera(
            "{%- set a = 1 -%}{%- set b = 2 -%}{\"a\": 1}"
        ));
    }

    #[test]
    fn test_classification_pure_expression_not_json() {
        assert!(!looks_like_json_with_tera("{{ fake_pdf(text=\"hello\") }}"));
    }

    #[test]
    fn test_classification_block_then_expression_not_json() {
        assert!(!looks_like_json_with_tera(
            "{%- set x = 1 -%}{{ fake_name() }}"
        ));
    }

    #[test]
    fn test_classification_plain_text_not_json() {
        assert!(!looks_like_json_with_tera("hello world"));
    }

    #[test]
    fn test_classification_tera_blocks_only_not_json() {
        assert!(!looks_like_json_with_tera(
            "{%- set x = 1 -%}{%- set y = 2 -%}"
        ));
    }

    #[test]
    fn test_classification_array_with_tera() {
        assert!(looks_like_json_with_tera(
            "[{% for i in items %}{{ i }}{% endfor %}]"
        ));
    }

    // ==========================================================================
    // has_newline_in_range (unit tests)
    // ==========================================================================

    #[test]
    fn test_has_newline_in_range() {
        let chars: Vec<char> = "abc\ndef".chars().collect();
        assert!(!has_newline_in_range(&chars, 0, 3));
        assert!(has_newline_in_range(&chars, 0, 4));
        assert!(has_newline_in_range(&chars, 3, 4));
        assert!(!has_newline_in_range(&chars, 4, 7));
    }

    #[test]
    fn test_has_newline_out_of_bounds_clamped() {
        let chars: Vec<char> = "abc".chars().collect();
        assert!(!has_newline_in_range(&chars, 0, 100));
    }

    // ==========================================================================
    // is_always_structural (unit tests)
    // ==========================================================================

    #[test]
    fn test_always_structural_keywords() {
        assert!(is_always_structural("for"));
        assert!(is_always_structural("endfor"));
        assert!(is_always_structural("set"));
        assert!(is_always_structural("block"));
        assert!(is_always_structural("endblock"));
        assert!(is_always_structural("macro"));
        assert!(is_always_structural("endmacro"));
        assert!(is_always_structural("else"));
        assert!(is_always_structural("elif"));
        // NOT always structural
        assert!(!is_always_structural("if"));
        assert!(!is_always_structural("endif"));
        assert!(!is_always_structural("include"));
        assert!(!is_always_structural("extends"));
    }

    // ==========================================================================
    // extract_tera_block (unit tests)
    // ==========================================================================

    #[test]
    fn test_extract_tera_block_expression() {
        let chars: Vec<char> = "{{ name }}rest".chars().collect();
        let result = extract_tera_block(&chars, 0);
        assert_eq!(result, Some(("{{ name }}".to_string(), 10)));
    }

    #[test]
    fn test_extract_tera_block_tag() {
        let chars: Vec<char> = "{% for i in items %}rest".chars().collect();
        let result = extract_tera_block(&chars, 0);
        assert_eq!(result, Some(("{% for i in items %}".to_string(), 20)));
    }

    #[test]
    fn test_extract_tera_block_comment() {
        let chars: Vec<char> = "{# hello #}rest".chars().collect();
        let result = extract_tera_block(&chars, 0);
        assert_eq!(result, Some(("{# hello #}".to_string(), 11)));
    }

    #[test]
    fn test_extract_tera_block_unclosed_returns_none() {
        let chars: Vec<char> = "{{ name".chars().collect();
        assert_eq!(extract_tera_block(&chars, 0), None);
    }

    #[test]
    fn test_extract_tera_block_not_tera() {
        let chars: Vec<char> = "{\"key\": 1}".chars().collect();
        assert_eq!(extract_tera_block(&chars, 0), None);
    }

    // ==========================================================================
    // Nested for loops
    // ==========================================================================

    #[test]
    fn test_nested_for_loops() {
        let input = r#"{"matrix":[{% for row in rows %}[{% for col in cols %}{{ col }}{% if not loop.last %},{% endif %}{% endfor %}]{% if not loop.last %},{% endif %}{% endfor %}]}"#;
        let result = format_body(input);

        // Both for blocks should be on their own lines
        let lines: Vec<&str> = result.lines().collect();
        let for_lines: Vec<&&str> = lines
            .iter()
            .filter(|l| l.trim().starts_with("{% for"))
            .collect();
        assert_eq!(
            for_lines.len(),
            2,
            "Should have 2 for lines. Got:\n{result}"
        );

        let endfor_lines: Vec<&&str> = lines
            .iter()
            .filter(|l| l.trim().starts_with("{% endfor"))
            .collect();
        assert_eq!(
            endfor_lines.len(),
            2,
            "Should have 2 endfor lines. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(
            result, second,
            "NOT IDEMPOTENT\n--- FIRST ---\n{result}\n--- SECOND ---\n{second}\n"
        );
    }

    // ==========================================================================
    // Mixed structural blocks: set + for + if
    // ==========================================================================

    #[test]
    fn test_set_for_if_combined() {
        // Realistic pattern: set variables, loop, conditional rendering
        let input = r#"{%- set total = 50 -%}{%- set limit = 10 -%}{"total": {{ total }}, "items": [{% for i in range(start=0, end=limit) %}{%- if i < total -%}{"id": {{ i }}, "name": "{{ fake_name() }}"}{% if not loop.last %},{% endif %}{%- endif -%}{% endfor %}]}"#;
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        // 2 set blocks
        let set_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("set")).collect();
        assert_eq!(
            set_lines.len(),
            2,
            "Should have 2 set lines. Got:\n{result}"
        );

        // for on its own line
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% for")),
            "for on own line. Got:\n{result}"
        );

        // endfor on its own line
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% endfor")),
            "endfor on own line. Got:\n{result}"
        );

        // Inline comma pattern preserved
        assert!(
            result.contains("}{% if not loop.last %},{% endif %}"),
            "Inline comma preserved. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    // ==========================================================================
    // Edge cases
    // ==========================================================================

    #[test]
    fn test_json_array_at_root() {
        assert_formats_to(
            r#"[{"id":"{{ uuid() }}"},{"id":"{{ uuid() }}"}]"#,
            "\
[\
\n  {\
\n    \"id\": \"{{ uuid() }}\"\
\n  },\
\n  {\
\n    \"id\": \"{{ uuid() }}\"\
\n  }\
\n]",
        );
    }

    #[test]
    fn test_tera_expression_as_array_element() {
        assert_formats_to(
            r#"{"values":[{{ x }},{{ y }},{{ z }}]}"#,
            "\
{\
\n  \"values\": [\
\n    {{ x }},\
\n    {{ y }},\
\n    {{ z }}\
\n  ]\
\n}",
        );
    }

    #[test]
    fn test_single_field_object() {
        assert_formats_to(
            r#"{"id":"{{ uuid() }}"}"#,
            "{\n  \"id\": \"{{ uuid() }}\"\n}",
        );
    }

    #[test]
    fn test_only_tera_expression_in_object_value() {
        // Tera expression as a bare value (not in string quotes)
        assert_formats_to(
            r#"{"size":{{ fake_file_size(min=1024, max=10485760) }}}"#,
            "{\n  \"size\": {{ fake_file_size(min=1024, max=10485760) }}\n}",
        );
    }

    #[test]
    fn test_tera_expression_with_filters() {
        assert_formats_to(
            r#"{"score":{{ fake_float(min=0.5, max=1.0) | round(precision=2) }}}"#,
            "{\n  \"score\": {{ fake_float(min=0.5, max=1.0) | round(precision=2) }}\n}",
        );
    }

    #[test]
    fn test_multiple_tera_in_one_string_value() {
        // Tera expressions inside a JSON string value (not normalized)
        assert_formats_to(
            r#"{"url":"{{ base_url }}/users?limit={{ limit }}&offset={{ offset }}"}"#,
            "{\n  \"url\": \"{{ base_url }}/users?limit={{ limit }}&offset={{ offset }}\"\n}",
        );
    }

    // ==========================================================================
    // Comprehensive idempotency tests with all real-world patterns
    // ==========================================================================

    #[test]
    fn test_idempotency_all_patterns() {
        let patterns = vec![
            // Pure JSON
            r#"{"id":"123","name":"test"}"#,
            // JSON with Tera expressions
            r#"{"id":"{{ uuid() }}","name":"{{ fake_name() }}"}"#,
            // Set blocks + JSON
            r#"{%- set x = 1 -%}{%- set y = 2 -%}{"x": {{ x }}, "y": {{ y }}}"#,
            // For loop with conditional comma
            r#"{"items":[{% for i in range(end=3) %}{"id":{{ i }}}{% if not loop.last %},{% endif %}{% endfor %}]}"#,
            // Structural if/else/endif
            "{% if active %}\n{\"status\": \"ok\"}\n{% else %}\n{\"status\": \"inactive\"}\n{% endif %}",
            // Inline conditional value
            r#"{"next":{% if has_more %}"cursor"{% else %}null{% endif %}}"#,
            // Nested objects
            r#"{"user":{"profile":{"name":"{{ fake_name() }}"}}}"#,
            // Array at root
            r#"[{"id":1},{"id":2},{"id":3}]"#,
            // Empty containers
            r#"{"data":{},"list":[]}"#,
            // Comment before JSON
            r#"{# API response #}{"id":"123"}"#,
            // Plain text with Tera
            "Hello {{ name }}, welcome!",
            // File generation
            r#"{{ fake_pdf(text="hello") }}"#,
        ];

        for (i, pattern) in patterns.iter().enumerate() {
            let first = format_body(pattern);
            let second = format_body(&first);
            assert_eq!(
                first, second,
                "Pattern {i} NOT IDEMPOTENT.\nInput: {pattern}\nFirst:\n{first}\nSecond:\n{second}"
            );
        }
    }

    // ==========================================================================
    // Real-world mock body from file-generation.yaml
    // ==========================================================================

    #[test]
    fn test_user_profile_with_data_uri() {
        assert_formats_to(
            r#"{"id":"{{ captures.user_id }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}","avatar":"{{ fake_png_data_uri(width=200, height=200, color='#FF5722') }}","created_at":"{{ now() | date(format='%Y-%m-%dT%H:%M:%SZ') }}"}"#,
            "\
{\
\n  \"id\": \"{{ captures.user_id }}\",\
\n  \"name\": \"{{ fake_name() }}\",\
\n  \"email\": \"{{ fake_email() }}\",\
\n  \"avatar\": \"{{ fake_png_data_uri(width=200, height=200, color='#FF5722') }}\",\
\n  \"created_at\": \"{{ now() | date(format='%Y-%m-%dT%H:%M:%SZ') }}\"\
\n}",
        );
    }

    // ==========================================================================
    // Search result pattern with facets
    // ==========================================================================

    #[test]
    fn test_search_results_nested_objects_and_arrays() {
        let input = r#"{"query":"{{ query.q }}","results":[{% for i in range(start=0, end=5) %}{"id":"{{ fake_uuid() }}","title":"{{ fake_sentence(words=5) }}","highlights":["{{ fake_word() }}","{{ fake_word() }}"]}{% if not loop.last %},{% endif %}{% endfor %}],"facets":{"types":{"document":{{ fake_number(min=10, max=100) }},"file":{{ fake_number(min=5, max=50) }}}}}"#;
        let result = format_body(input);

        // for on own line
        let lines: Vec<&str> = result.lines().collect();
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% for")),
            "for on own line. Got:\n{result}"
        );
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% endfor")),
            "endfor on own line. Got:\n{result}"
        );

        // Inline comma preserved
        assert!(
            result.contains("}{% if not loop.last %},{% endif %}"),
            "Inline comma. Got:\n{result}"
        );

        // Nested arrays formatted
        assert!(
            result.contains("\"highlights\": ["),
            "highlights array. Got:\n{result}"
        );

        // Nested facets object
        assert!(
            result.contains("\"facets\": {"),
            "facets object. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    // ==========================================================================
    // Structural if wrapping JSON content with newlines
    // ==========================================================================

    #[test]
    fn test_structural_if_wrapping_for_loop() {
        let input = "{\n  \"items\": [\n    {% for i in range(end=3) %}\n    {% if i < total %}\n    {\"id\": {{ i }}}\n    {% endif %}\n    {% endfor %}\n  ]\n}";
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% if")),
            "Structural if. Got:\n{result}"
        );
        assert!(
            lines.iter().any(|l| l.trim().starts_with("{% endif")),
            "Structural endif. Got:\n{result}"
        );

        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    // ==========================================================================
    // For loop with set inside (set at non-zero depth)
    // ==========================================================================

    #[test]
    fn test_set_inside_for_loop() {
        let input = r#"{"items":[{% for i in range(end=3) %}{%- set name = fake_name() -%}{"id":{{ i }},"name":"{{ name }}"}{% if not loop.last %},{% endif %}{% endfor %}]}"#;
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        // set block inside the loop should be on its own line
        let set_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("set")).collect();
        assert_eq!(set_lines.len(), 1, "Should have 1 set line. Got:\n{result}");
        assert!(
            set_lines[0].trim().starts_with("{%"),
            "Set should start its line. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    // ==========================================================================
    // Tera block indentation tests
    // ==========================================================================

    #[test]
    fn test_if_block_indents_content() {
        // Use JSON-with-Tera with newlines to trigger structural formatting
        let input = "{%- set total = 25 -%}\n{%- if total > 10 %}\n{%- set result = \"high\" -%}\n{\"value\": {{ result }}}\n{% endif -%}";
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();
        let if_line = lines
            .iter()
            .position(|l| l.contains("if total"))
            .unwrap_or_else(|| panic!("No if line. Got:\n{result}"));
        let set_line = lines
            .iter()
            .position(|l| l.contains("set result"))
            .unwrap_or_else(|| panic!("No set result line. Got:\n{result}"));
        let endif_line = lines
            .iter()
            .position(|l| l.contains("endif"))
            .unwrap_or_else(|| panic!("No endif line. Got:\n{result}"));

        // Set inside if should be indented more than if
        let if_indent = lines[if_line]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let set_indent = lines[set_line]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let endif_indent = lines[endif_line]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        assert!(
            set_indent > if_indent,
            "Content inside if should be indented. if_indent={if_indent}, set_indent={set_indent}. Got:\n{result}"
        );
        assert_eq!(
            if_indent, endif_indent,
            "endif should align with if. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_nested_if_blocks() {
        // Use JSON-with-Tera with newlines to trigger structural formatting
        let input = "{%- if outer %}\n{%- if inner %}\n{%- set value = 1 -%}\n{\"value\": {{ value }}}\n{% endif -%}\n{% endif -%}";
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        // Find indentation levels (handle trim markers)
        let outer_if_indent = lines
            .iter()
            .find(|l| l.contains("if outer"))
            .unwrap_or_else(|| panic!("No outer if. Got:\n{result}"))
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let inner_if_indent = lines
            .iter()
            .find(|l| l.contains("if inner"))
            .unwrap_or_else(|| panic!("No inner if. Got:\n{result}"))
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let set_indent = lines
            .iter()
            .find(|l| l.contains("set value"))
            .unwrap_or_else(|| panic!("No set value. Got:\n{result}"))
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        // Each level should be indented 2 more spaces
        assert_eq!(
            inner_if_indent,
            outer_if_indent + 2,
            "Inner if should be indented. Got:\n{result}"
        );
        assert_eq!(
            set_indent,
            inner_if_indent + 2,
            "Set inside inner if should be indented. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_for_loop_indents_content() {
        let input = r#"{"items":[{% for i in range(end=3) %}{"id":{{ i }}}{% endfor %}]}"#;
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        let for_line_idx = lines.iter().position(|l| l.contains("{% for")).unwrap();
        let id_line_idx = lines.iter().position(|l| l.contains("\"id\"")).unwrap();
        let endfor_line_idx = lines.iter().position(|l| l.contains("{% endfor")).unwrap();

        let for_indent = lines[for_line_idx]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let id_indent = lines[id_line_idx]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let endfor_indent = lines[endfor_line_idx]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        // Content inside for should be indented
        assert!(
            id_indent > for_indent,
            "Content inside for should be indented. for_indent={for_indent}, id_indent={id_indent}. Got:\n{result}"
        );
        // endfor should align with for
        assert_eq!(
            for_indent, endfor_indent,
            "endfor should align with for. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_if_else_endif_alignment() {
        // Use JSON-with-Tera with newlines to trigger structural formatting
        let input = "{\n  \"result\": \n  {% if condition %}\n  {%- set a = 1 -%}\n  {{ a }}\n  {% else %}\n  {%- set a = 2 -%}\n  {{ a }}\n  {% endif %}\n}";
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        let if_indent = lines
            .iter()
            .find(|l| l.contains("{% if"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let else_indent = lines
            .iter()
            .find(|l| l.contains("{% else"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let endif_indent = lines
            .iter()
            .find(|l| l.contains("{% endif"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        // if, else, and endif should all be at the same indentation level
        assert_eq!(
            if_indent, else_indent,
            "else should align with if. Got:\n{result}"
        );
        assert_eq!(
            if_indent, endif_indent,
            "endif should align with if. Got:\n{result}"
        );

        // Content inside each branch should be indented
        let set_a_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.trim().starts_with("{%") && l.contains("set a"))
            .copied()
            .collect();
        assert_eq!(
            set_a_lines.len(),
            2,
            "Should have 2 set statements. Got:\n{result}"
        );

        for set_line in set_a_lines {
            let set_indent = set_line.chars().take_while(|c| c.is_whitespace()).count();
            assert!(
                set_indent > if_indent,
                "Content should be indented. Got:\n{result}"
            );
        }

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_elif_alignment() {
        let input = r#"{% if x > 10 %}{%- set result = "high" -%}{% elif x > 5 %}{%- set result = "medium" -%}{% else %}{%- set result = "low" -%}{% endif %}"#;
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        let if_indent = lines
            .iter()
            .find(|l| l.contains("{% if"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let elif_indent = lines
            .iter()
            .find(|l| l.contains("{% elif"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let else_indent = lines
            .iter()
            .find(|l| l.contains("{% else"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        let endif_indent = lines
            .iter()
            .find(|l| l.contains("{% endif"))
            .unwrap()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();

        // All should be at the same level
        assert_eq!(
            if_indent, elif_indent,
            "elif should align with if. Got:\n{result}"
        );
        assert_eq!(
            if_indent, else_indent,
            "else should align with if. Got:\n{result}"
        );
        assert_eq!(
            if_indent, endif_indent,
            "endif should align with if. Got:\n{result}"
        );

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_complex_nesting_for_with_if() {
        // Input with newlines to trigger structural formatting
        let input = "{\n  \"items\": [\n    {% for i in range(end=5) %}\n    {% if i % 2 == 0 %}\n    {\"id\": {{ i }}, \"even\": true}\n    {% else %}\n    {\"id\": {{ i }}, \"even\": false}\n    {% endif %}\n    {% if not loop.last %},{% endif %}\n    {% endfor %}\n  ]\n}";
        let result = format_body(input);

        let lines: Vec<&str> = result.lines().collect();

        // for should be at base indentation
        let for_line = lines.iter().find(|l| l.contains("{% for")).unwrap();
        let for_indent = for_line.chars().take_while(|c| c.is_whitespace()).count();

        // if inside for should be indented more
        let if_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.trim().starts_with("{% if") && !l.contains("not loop.last"))
            .copied()
            .collect();
        assert!(!if_lines.is_empty(), "Should have if block. Got:\n{result}");

        let if_indent = if_lines[0]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        assert!(
            if_indent > for_indent,
            "if inside for should be indented. for_indent={for_indent}, if_indent={if_indent}. Got:\n{result}"
        );

        // JSON content inside if should be indented even more
        let id_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.contains("\"id\"") && !l.contains("{% if"))
            .copied()
            .collect();
        for id_line in id_lines {
            let id_indent = id_line.chars().take_while(|c| c.is_whitespace()).count();
            assert!(
                id_indent > if_indent,
                "JSON inside if should be indented. if_indent={if_indent}, id_indent={id_indent}. Got:\n{result}"
            );
        }

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }

    #[test]
    fn test_user_example_inbox_pagination() {
        // The exact example from the user's request
        let input = r#"{%- set total = store_get_or_set(key="inbox.total", default="25") | int -%}
      {%- set page = query.page | default(value="1") | int -%}
      {%- set limit = query.limit | default(value="10") | int -%}
      {%- set offset = (page - 1) * limit -%}
      {%- set remaining = total - offset -%}
      {%- set count_on_page = remaining -%}
      {%- if count_on_page > limit %}
      {% set count_on_page = limit %}
      {% endif -%}
      {%- if count_on_page < 0 %}
      {% set count_on_page = 0 %}
      {% endif -%}
      {"count":{{ count_on_page }}}"#;

        let result = format_body(input);
        let lines: Vec<&str> = result.lines().collect();

        // Find the if blocks and their content (handle trim markers)
        let if_blocks: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("if count_on_page"))
            .map(|(i, l)| (i, *l))
            .collect();

        assert_eq!(
            if_blocks.len(),
            2,
            "Should have 2 if blocks. Got:\n{result}"
        );

        for (if_idx, if_line) in if_blocks {
            let if_indent = if_line.chars().take_while(|c| c.is_whitespace()).count();

            // Find the set statement inside this if block (next line)
            if if_idx + 1 < lines.len() {
                let next_line = lines[if_idx + 1];
                if next_line.contains("{% set count_on_page") {
                    let set_indent = next_line.chars().take_while(|c| c.is_whitespace()).count();
                    assert!(
                        set_indent > if_indent,
                        "Set inside if should be indented more than if. if_indent={if_indent}, set_indent={set_indent}. Got:\n{result}"
                    );
                }
            }

            // Find the endif for this if block
            if if_idx + 2 < lines.len() {
                let endif_line = lines[if_idx + 2];
                if endif_line.contains("{% endif") {
                    let endif_indent = endif_line.chars().take_while(|c| c.is_whitespace()).count();
                    assert_eq!(
                        if_indent, endif_indent,
                        "endif should align with if. Got:\n{result}"
                    );
                }
            }
        }

        // Idempotent
        let second = format_body(&result);
        assert_eq!(result, second, "NOT IDEMPOTENT\n{result}\nvs\n{second}");
    }
}
