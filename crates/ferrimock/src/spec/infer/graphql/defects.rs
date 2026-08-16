//! Malformed SDL that real schemas ship anyway.
//!
//! A single-line description is a normal GraphQL string, so a `"` inside it
//! has to be escaped or the description has to be a block string. Generators
//! that build descriptions by string interpolation get this wrong constantly —
//! `"The MIME type (e.g., "text/plain")."` is rejected by every conforming
//! parser, graphql-js included.
//!
//! Being able to *name* that beats failing at the byte offset where the
//! grammar gave up. Repairing it is offered, never assumed: a tool that
//! silently rewrites its input is a tool you cannot trust with the next file.

use std::fmt;

/// A malformation worth naming, with the line it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdlDefect {
    pub line: usize,
    pub kind: DefectKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefectKind {
    /// A single-line description containing unescaped `"`.
    UnescapedQuoteInDescription,
}

impl fmt::Display for SdlDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            DefectKind::UnescapedQuoteInDescription => write!(
                f,
                "line {}: description contains unescaped quotes — escape them as \\\" or use a \
                 \"\"\"block string\"\"\"\n    {}",
                self.line,
                self.text.trim()
            ),
        }
    }
}

/// Find malformations this module knows how to explain.
#[must_use]
pub fn find_defects(source: &str) -> Vec<SdlDefect> {
    scan(source).0
}

/// Repair what can be repaired, returning the new source and what was changed.
///
/// Repair is per line and mechanical: the inner quotes are escaped, which is
/// what the author meant and what a conforming generator would have emitted.
#[must_use]
pub fn repair(source: &str) -> (String, Vec<SdlDefect>) {
    let (defects, repaired) = scan(source);
    (repaired, defects)
}

fn scan(source: &str) -> (Vec<SdlDefect>, String) {
    let mut defects = Vec::new();
    let mut out = String::with_capacity(source.len());
    let mut in_block_string = false;

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        // Block strings may contain anything, including lines that look like
        // descriptions; count the delimiters rather than guessing.
        let delimiters = trimmed.matches("\"\"\"").count();
        if delimiters % 2 == 1 {
            in_block_string = !in_block_string;
            push_line(&mut out, line, source, index);
            continue;
        }
        if in_block_string || trimmed.starts_with("\"\"\"") {
            push_line(&mut out, line, source, index);
            continue;
        }

        match malformed_description(line) {
            Some(fixed) => {
                defects.push(SdlDefect {
                    line: index + 1,
                    kind: DefectKind::UnescapedQuoteInDescription,
                    text: trimmed.to_string(),
                });
                push_line(&mut out, &fixed, source, index);
            }
            None => push_line(&mut out, line, source, index),
        }
    }

    if source.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    (defects, out)
}

fn push_line(out: &mut String, line: &str, source: &str, index: usize) {
    if index > 0 || source.starts_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
}

/// A line that is a whole single-line description with unescaped inner quotes,
/// and the same line with them escaped.
fn malformed_description(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    let indent_len = trimmed.len() - trimmed.trim_start().len();
    let (indent, body) = trimmed.split_at(indent_len);

    if body.len() < 2 || !body.starts_with('"') || !body.ends_with('"') {
        return None;
    }

    let inner = body.get(1..body.len() - 1)?;
    if !has_unescaped_quote(inner) {
        return None;
    }

    let escaped = escape_quotes(inner);
    Some(format!("{indent}\"{escaped}\""))
}

fn has_unescaped_quote(inner: &str) -> bool {
    let mut escaped = false;
    for ch in inner.chars() {
        match ch {
            '\\' => escaped = !escaped,
            '"' if !escaped => return true,
            _ => escaped = false,
        }
    }
    false
}

fn escape_quotes(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len() + 4);
    let mut escaped = false;
    for ch in inner.chars() {
        match ch {
            '\\' => {
                escaped = !escaped;
                out.push(ch);
            }
            '"' if !escaped => out.push_str("\\\""),
            _ => {
                escaped = false;
                out.push(ch);
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn an_unescaped_quote_in_a_description_is_found_and_named() {
        let source = "type A {\n  \"The MIME type (e.g., \"text/plain\").\"\n  f: String\n}";
        let defects = find_defects(source);
        assert_eq!(defects.len(), 1);
        assert_eq!(defects[0].line, 2);
        assert!(defects[0].to_string().contains("line 2"));
    }

    #[test]
    fn repairing_makes_it_parse() {
        let source = "type A {\n  \"The MIME type (e.g., \"text/plain\").\"\n  f: String\n}";
        let (repaired, defects) = repair(source);
        assert_eq!(defects.len(), 1);
        assert!(repaired.contains(r#"\"text/plain\""#));
        assert!(
            super::super::sdl::parse_sdl(&repaired).is_ok(),
            "the repaired source should parse: {repaired}"
        );
    }

    #[test]
    fn a_well_formed_schema_is_left_exactly_as_it_was() {
        let source = "type A {\n  \"A plain description\"\n  f: String\n}\n";
        let (repaired, defects) = repair(source);
        assert!(defects.is_empty());
        assert_eq!(repaired, source);
    }

    #[test]
    fn an_already_escaped_quote_is_not_touched() {
        let source = "\"He said \\\"hello\\\"\"\ntype A { f: String }";
        let (repaired, defects) = repair(source);
        assert!(defects.is_empty());
        assert_eq!(repaired, source);
    }

    #[test]
    fn block_strings_are_left_alone() {
        let source = "\"\"\"\nA block string with \"quotes\" in it\n\"\"\"\ntype A { f: String }";
        let (repaired, defects) = repair(source);
        assert!(
            defects.is_empty(),
            "a block string may contain quotes; it is not malformed"
        );
        assert_eq!(repaired, source);
    }

    #[test]
    fn a_single_line_block_string_is_left_alone() {
        let source = "\"\"\"A \"quoted\" thing\"\"\"\ntype A { f: String }";
        let (_, defects) = repair(source);
        assert!(defects.is_empty());
    }

    #[test]
    fn a_field_default_is_not_a_description() {
        let source = "type A { f(x: String = \"a\\\"b\"): String }";
        let (repaired, defects) = repair(source);
        assert!(defects.is_empty());
        assert_eq!(repaired, source);
    }

    #[test]
    fn every_occurrence_is_reported_not_just_the_first() {
        let source =
            "\"a \"b\" c\"\ntype A { f: String }\n\"d \"e\" f\"\ntype B { g: String }";
        let defects = find_defects(source);
        assert_eq!(defects.len(), 2);
        assert_eq!(defects[0].line, 1);
        assert_eq!(defects[1].line, 3);
    }
}
