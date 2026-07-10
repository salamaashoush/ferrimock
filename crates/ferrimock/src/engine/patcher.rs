//! Response patching for modifying upstream responses

use super::types::PatchOperation;
use bytes::Bytes;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use http::Response;
use http::header::{self, HeaderName, HeaderValue};
use std::io::{Read, Write};

/// Get a human-readable type name for a JSON value
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Decompress gzip-encoded data if needed.
/// Returns Some(decompressed) if data was gzipped and successfully decompressed, None otherwise.
/// Avoids cloning the input when decompression is not needed.
fn try_decompress_gzip(data: &Bytes, content_encoding: Option<&str>) -> Option<Bytes> {
    let is_gzipped = content_encoding.is_some_and(|e| e.contains("gzip"));

    if !is_gzipped {
        return None;
    }

    // Try to decompress
    let mut decoder = GzDecoder::new(&data[..]);
    let mut decompressed = Vec::new();
    match decoder.read_to_end(&mut decompressed) {
        Ok(_) => {
            tracing::debug!(
                "Decompressed gzip body for patching: {} -> {} bytes",
                data.len(),
                decompressed.len()
            );
            Some(Bytes::from(decompressed))
        }
        Err(e) => {
            tracing::warn!("Failed to decompress gzip data for patching: {}", e);
            None
        }
    }
}

/// Re-compress data using gzip
fn recompress_gzip(data: &Bytes) -> Result<Bytes, std::io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    let compressed = encoder.finish()?;
    tracing::debug!(
        "Re-compressed patched body: {} -> {} bytes",
        data.len(),
        compressed.len()
    );
    Ok(Bytes::from(compressed))
}

/// Response patcher for transforming responses
#[derive(Debug, Clone)]
pub struct ResponsePatcher {
    /// Patch operations to apply
    operations: Vec<PatchOperation>,
}

impl ResponsePatcher {
    /// Create a new response patcher with the given operations
    pub fn new(operations: Vec<PatchOperation>) -> Self {
        Self { operations }
    }

    /// Apply patches to a response.
    ///
    /// Optimized to batch consecutive JSON operations (JsonPatch, JsonPath) so the body
    /// is parsed from bytes once and serialized back once per batch, instead of per-operation.
    ///
    /// If `patch_context` is provided, string values containing `{{` or `{%` will be
    /// rendered as Tera templates with access to request context (captures, vars, headers,
    /// body) and upstream response data (response.status, response.headers, response.body_json).
    #[allow(clippy::indexing_slicing, clippy::string_slice, clippy::unwrap_used)] // JSON path navigation uses bounds-checked indexing
    pub fn apply(
        &self,
        mut response: Response<Bytes>,
        patch_context: Option<&crate::types::PatchContext>,
    ) -> Result<Response<Bytes>, crate::FerrimockError> {
        // Check for gzip compression and decompress if needed.
        // Uses Option to avoid cloning body bytes when not compressed.
        let content_encoding = response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok());
        let decompressed = try_decompress_gzip(response.body(), content_encoding);
        let was_compressed = decompressed.is_some();

        // Use decompressed body if available, otherwise work directly with response body.
        // We take ownership via clone only when we actually need to modify the body.
        let mut body = decompressed.unwrap_or_else(|| response.body().clone());

        // Extract headers
        let headers = response.headers_mut();

        // Process operations with batched JSON handling.
        // Consecutive JSON operations (JsonPatch, JsonPath) are grouped and applied to a single
        // parsed JSON value, avoiding repeated parse/serialize cycles.
        let mut i = 0;
        while i < self.operations.len() {
            match &self.operations[i] {
                // JSON operations: batch consecutive ones together
                PatchOperation::JsonPatch(_) | PatchOperation::JsonPath { .. } => {
                    // Parse body as JSON once for the entire batch
                    let mut json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
            let preview_len = body.len().min(200);
            let body_preview = String::from_utf8_lossy(&body[..preview_len]);
            crate::mp_err!(
              "Failed to parse JSON for patching: {}. Body length: {}, preview: {:?}",
              e,
              body.len(),
              body_preview
            )
          })?;

                    // Apply all consecutive JSON operations to the parsed value
                    while i < self.operations.len() {
                        match &self.operations[i] {
                            PatchOperation::JsonPatch(patch) => {
                                json_patch::patch(&mut json, patch)?;
                                i += 1;
                            }
                            PatchOperation::JsonPath { path, value } => {
                                // Render template expressions in path and value if patch_context is available
                                let rendered_path = render_if_template(path, patch_context);
                                let rendered_value =
                                    render_json_value_templates(value, patch_context);
                                apply_jsonpath_patch(&mut json, &rendered_path, rendered_value)
                                    .map_err(|e| {
                                        crate::mp_err!(
                                            "JSONPath patch failed for '{rendered_path}': {e}"
                                        )
                                    })?;
                                i += 1;
                            }
                            // Non-JSON operation breaks the batch
                            _ => break,
                        }
                    }

                    // Serialize back to bytes once for the entire batch
                    body = Bytes::from(serde_json::to_vec(&json)?);
                }
                PatchOperation::RegexReplace {
                    pattern,
                    replacement,
                } => {
                    // Convert body to string
                    let body_str = String::from_utf8(body.to_vec())?;

                    // Render template expressions in replacement if patch_context is available
                    let rendered_replacement = render_if_template(replacement, patch_context);

                    // Apply regex replacement
                    let replaced = pattern.replace_all(&body_str, &*rendered_replacement);

                    // Convert back to bytes
                    body = Bytes::from(replaced.to_string());
                    i += 1;
                }
                PatchOperation::HeaderAdd { name, value } => {
                    // Render template expressions in name and value if patch_context is available
                    let rendered_name = render_if_template(name, patch_context);
                    let rendered_value = render_if_template(value, patch_context);
                    let header_name = HeaderName::try_from(&*rendered_name)?;
                    let header_value = HeaderValue::try_from(&*rendered_value)?;
                    headers.insert(header_name, header_value);
                    i += 1;
                }
                PatchOperation::HeaderRemove { name } => {
                    // Render template expressions in name if patch_context is available
                    let rendered_name = render_if_template(name, patch_context);
                    let header_name = HeaderName::try_from(&*rendered_name)?;
                    headers.remove(&header_name);
                    i += 1;
                }
            }
        }

        // Re-compress if the original was compressed
        let final_body = if was_compressed {
            match recompress_gzip(&body) {
                Ok(compressed) => compressed,
                Err(e) => {
                    tracing::warn!(
                        "Failed to re-compress patched body, returning uncompressed: {}",
                        e
                    );
                    // Remove Content-Encoding header since we're returning uncompressed
                    headers.remove(header::CONTENT_ENCODING);
                    body
                }
            }
        } else {
            body
        };

        // Update Content-Length header with the new body size
        if let Ok(len_value) = HeaderValue::try_from(final_body.len().to_string()) {
            headers.insert(header::CONTENT_LENGTH, len_value);
        }

        // Build new response with patched body and headers
        let (parts, _) = response.into_parts();
        Ok(Response::from_parts(parts, final_body))
    }
}

/// Render a string as a Tera template if it contains template markers (`{{` or `{%`).
/// Returns the original string (as a `Cow`) when no template markers are present (zero-cost).
/// On render failure, logs a warning and returns the original string unchanged.
fn render_if_template<'a>(
    value: &'a str,
    patch_context: Option<&crate::types::PatchContext>,
) -> std::borrow::Cow<'a, str> {
    let Some(ctx) = patch_context else {
        return std::borrow::Cow::Borrowed(value);
    };
    if !value.contains("{{") && !value.contains("{%") {
        return std::borrow::Cow::Borrowed(value);
    }
    match crate::template::render_patch_template(value, ctx, None) {
        Ok(rendered) => std::borrow::Cow::Owned(rendered),
        Err(e) => {
            tracing::warn!(
                "Template render failed in patch value, using literal: {}",
                e
            );
            std::borrow::Cow::Borrowed(value)
        }
    }
}

/// Render template expressions inside a JSON value.
///
/// Only `Value::String` variants containing `{{` or `{%` are rendered. After rendering,
/// the result is re-parsed as JSON so that templates producing numbers, booleans, or objects
/// are inserted with the correct JSON type (e.g. `"{{ random_int(min=1, max=10) }}"` becomes
/// a JSON number). If re-parsing fails, the rendered string is kept as a `Value::String`.
fn render_json_value_templates(
    value: &serde_json::Value,
    patch_context: Option<&crate::types::PatchContext>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s)
            if patch_context.is_some() && (s.contains("{{") || s.contains("{%")) =>
        {
            // SAFETY: patch_context.is_some() guard above ensures this won't panic
            let Some(ctx) = patch_context else {
                return value.clone();
            };
            match crate::template::render_patch_template(s, ctx, None) {
                Ok(rendered) => {
                    // Try to parse as JSON (number, bool, object, array)
                    serde_json::from_str(&rendered)
                        .unwrap_or_else(|_| serde_json::Value::String(rendered))
                }
                Err(e) => {
                    tracing::warn!(
                        "Template render failed in patch JSON value, using literal: {}",
                        e
                    );
                    value.clone()
                }
            }
        }
        _ => value.clone(),
    }
}

/// Represents a parsed path segment - either a field name or a field with array index
#[derive(Debug)]
enum PathSegment<'a> {
    /// Simple field name like "user"
    Field(&'a str),
    /// Field with array index like `signers[0]`
    ArrayIndex(&'a str, usize),
    /// Field with wildcard for all array elements like `signers[*]`
    ArrayWildcard(&'a str),
    /// Field with multiple indices like `signers[0,2,4]`
    ArrayMultiIndex(&'a str, Vec<usize>),
    /// Field with range like "signers[1..3]" (exclusive end) or "signers[1..=3]" (inclusive end)
    ArrayRange(&'a str, usize, usize, bool), // field, start, end, inclusive
}

/// Parse a path segment to extract field name and optional array index
/// Examples:
/// - `"user"` -> Field("user")
/// - `"signers[0]"` -> ArrayIndex("signers", 0)
/// - `"items[12]"` -> ArrayIndex("items", 12)
/// - `"signers[*]"` -> ArrayWildcard("signers")
/// - `"signers[0,2,4]"` -> ArrayMultiIndex("signers", \[0, 2, 4\])
/// - `"signers[1..3]"` -> ArrayRange("signers", 1, 3, false) (exclusive)
/// - `"signers[1..=3]"` -> ArrayRange("signers", 1, 3, true) (inclusive)
#[allow(clippy::string_slice, clippy::indexing_slicing)] // Path segment parsing uses character-position-based slicing
fn parse_path_segment(segment: &str) -> Result<PathSegment<'_>, crate::FerrimockError> {
    if let Some(bracket_pos) = segment.find('[') {
        if !segment.ends_with(']') {
            return Err(crate::mp_err!(
                "Invalid array notation in path segment: {segment}"
            ));
        }
        let field = &segment[..bracket_pos];
        let index_str = &segment[bracket_pos + 1..segment.len() - 1];

        // Check for wildcard
        if index_str == "*" {
            return Ok(PathSegment::ArrayWildcard(field));
        }

        // Check for multi-index (comma-separated)
        if index_str.contains(',') {
            let indices: Result<Vec<usize>, _> = index_str
                .split(',')
                .map(|s| s.trim().parse::<usize>())
                .collect();
            let indices = indices.map_err(|_| {
                crate::mp_err!("Invalid multi-index '{index_str}' in path segment: {segment}")
            })?;
            return Ok(PathSegment::ArrayMultiIndex(field, indices));
        }

        // Check for inclusive range (..=)
        if let Some(range_pos) = index_str.find("..=") {
            let start: usize = index_str[..range_pos].trim().parse().map_err(|_| {
                crate::mp_err!("Invalid range start in '{index_str}' for segment: {segment}")
            })?;
            let end: usize = index_str[range_pos + 3..].trim().parse().map_err(|_| {
                crate::mp_err!("Invalid range end in '{index_str}' for segment: {segment}")
            })?;
            return Ok(PathSegment::ArrayRange(field, start, end, true));
        }

        // Check for exclusive range (..)
        if let Some(range_pos) = index_str.find("..") {
            let start: usize = index_str[..range_pos].trim().parse().map_err(|_| {
                crate::mp_err!("Invalid range start in '{index_str}' for segment: {segment}")
            })?;
            let end: usize = index_str[range_pos + 2..].trim().parse().map_err(|_| {
                crate::mp_err!("Invalid range end in '{index_str}' for segment: {segment}")
            })?;
            return Ok(PathSegment::ArrayRange(field, start, end, false));
        }

        // Simple numeric index
        let index: usize = index_str.parse().map_err(|_| {
            crate::mp_err!("Invalid array index '{index_str}' in path segment: {segment}")
        })?;
        Ok(PathSegment::ArrayIndex(field, index))
    } else {
        Ok(PathSegment::Field(segment))
    }
}

/// Check if a segment is a multi-element selector (wildcard, multi-index, or range)
fn is_multi_element_selector(segment: &PathSegment<'_>) -> bool {
    matches!(
        segment,
        PathSegment::ArrayWildcard(_)
            | PathSegment::ArrayMultiIndex(_, _)
            | PathSegment::ArrayRange(_, _, _, _)
    )
}

/// Format available keys from an object for error messages
fn format_available_keys(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let keys: Vec<_> = obj.keys().take(15).cloned().collect();
    if keys.is_empty() {
        "(empty object)".to_string()
    } else if keys.len() < obj.len() {
        format!(
            "[{}, ... and {} more]",
            keys.join(", "),
            obj.len() - keys.len()
        )
    } else {
        format!("[{}]", keys.join(", "))
    }
}

/// Navigate to a value using a path segment, returning a mutable reference.
/// Only handles simple navigation (Field and ArrayIndex).
/// Multi-element selectors (wildcard, multi-index, range) must be handled by the caller.
#[allow(clippy::indexing_slicing, clippy::expect_used)] // Array/field access bounds are validated before access
fn navigate_segment<'a>(
    current: &'a mut serde_json::Value,
    segment: &PathSegment<'_>,
    create_missing: bool,
) -> Result<&'a mut serde_json::Value, crate::FerrimockError> {
    match segment {
        PathSegment::Field(field) => {
            // Check type before mutable borrow
            let type_name = json_type_name(current);
            if let Some(obj) = current.as_object_mut() {
                if create_missing && !obj.contains_key(*field) {
                    obj.insert(field.to_string(), serde_json::json!({}));
                }
                if obj.contains_key(*field) {
                    // SAFETY: contains_key check above ensures key exists
                    Ok(obj
                        .get_mut(*field)
                        .expect("key exists after contains_key check"))
                } else {
                    let available = format_available_keys(obj);
                    Err(crate::mp_err!(
                        "Field '{field}' not found. Available keys: {available}"
                    ))
                }
            } else {
                Err(crate::mp_err!(
                    "Cannot navigate to field '{field}': current value is {type_name} (expected object)"
                ))
            }
        }
        PathSegment::ArrayIndex(field, index) => {
            // Check type before mutable borrow
            let type_name = json_type_name(current);
            if let Some(obj) = current.as_object_mut() {
                if !obj.contains_key(*field) {
                    let available = format_available_keys(obj);
                    return Err(crate::mp_err!(
                        "Field '{field}' not found. Available keys: {available}"
                    ));
                }
                // SAFETY: contains_key check above ensures key exists
                let arr_val = obj
                    .get_mut(*field)
                    .expect("key exists after contains_key check");
                let arr_type = json_type_name(arr_val);
                if let Some(arr) = arr_val.as_array_mut() {
                    let len = arr.len();
                    arr.get_mut(*index).ok_or_else(|| {
                        crate::mp_err!(
                            "Array index {index} out of bounds for '{field}' (length: {len})"
                        )
                    })
                } else {
                    Err(crate::mp_err!(
                        "Field '{field}' is {arr_type} (expected array)"
                    ))
                }
            } else {
                Err(crate::mp_err!(
                    "Cannot access '{field}[{index}]': current value is {type_name} (expected object)"
                ))
            }
        }
        // Multi-element selectors should never reach here - they're handled separately
        PathSegment::ArrayWildcard(_)
        | PathSegment::ArrayMultiIndex(_, _)
        | PathSegment::ArrayRange(_, _, _, _) => Err(crate::mp_err!(
            "Internal error: multi-element selector reached navigate_segment. Path segment: {segment:?}"
        )),
    }
}

/// Split a JSONPath by dots, but not dots inside brackets
/// e.g., "items[1..3].name" -> ["items[1..3]", "name"]
#[allow(clippy::string_slice)] // ASCII-only JSON path syntax, byte positions are safe
fn split_jsonpath(path: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut bracket_depth = 0;

    for (i, c) in path.char_indices() {
        match c {
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            '.' if bracket_depth == 0 => {
                if i > start {
                    parts.push(&path[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
    }

    // Add the last segment
    if start < path.len() {
        parts.push(&path[start..]);
    }

    parts
}

/// Apply a JSONPath-style patch to a JSON value
/// Supports paths like:
/// - `$.user.name`
/// - `$.doc.signers[0].language`
/// - `$.items[2].value`
/// - `$.signers[*].language` (wildcard - applies to all array elements)
/// - `$.items[0,2,4].value` (multi-index)
/// - `$.items[1..3].value` (range, exclusive)
/// - `$.items[1..=3].value` (range, inclusive)
#[allow(clippy::indexing_slicing, clippy::unwrap_used)] // JSON path operations with validated indices
fn apply_jsonpath_patch(
    json: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), crate::FerrimockError> {
    // Remove leading $. if present
    let path = path.strip_prefix("$.").unwrap_or(path);
    let path = path.strip_prefix('$').unwrap_or(path);

    // Split path by dots (but not dots inside brackets like [1..3])
    let parts: Vec<&str> = split_jsonpath(path);

    if parts.is_empty() {
        return Err(crate::mp_err!("Empty JSONPath"));
    }

    // Parse all segments
    let segments: Vec<PathSegment<'_>> = parts
        .iter()
        .map(|p| parse_path_segment(p))
        .collect::<Result<Vec<_>, _>>()?;

    // Check if any segment is a wildcard, multi-index, or range - requires special handling
    for (i, segment) in segments.iter().enumerate() {
        // Get indices to apply based on segment type
        let (field, indices): (&str, Vec<usize>) = match segment {
            PathSegment::ArrayWildcard(field) => {
                // Navigate to get array length first
                let mut current = json.clone();
                for seg in segments.iter().take(i) {
                    if let Some(obj) = current.as_object() {
                        match seg {
                            PathSegment::Field(f) => {
                                current = obj.get(*f).cloned().unwrap_or(serde_json::Value::Null);
                            }
                            PathSegment::ArrayIndex(f, idx) => {
                                if let Some(arr) = obj.get(*f).and_then(|v| v.as_array()) {
                                    current =
                                        arr.get(*idx).cloned().unwrap_or(serde_json::Value::Null);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let arr_len = current
                    .as_object()
                    .and_then(|o| o.get(*field))
                    .and_then(|v| v.as_array())
                    .map_or(0, std::vec::Vec::len);
                (*field, (0..arr_len).collect())
            }
            PathSegment::ArrayMultiIndex(field, indices) => (*field, indices.clone()),
            PathSegment::ArrayRange(field, start, end, inclusive) => {
                let end_idx = if *inclusive { *end + 1 } else { *end };
                (*field, (*start..end_idx).collect())
            }
            _ => continue, // Not a multi-element selector, skip
        };

        // Navigate to the parent containing the array
        let mut current = json;
        for (seg_idx, seg) in segments.iter().take(i).enumerate() {
            current = navigate_segment(current, seg, true).map_err(|e| {
                let path_so_far = parts[..=seg_idx].join(".");
                crate::mp_err!("At '{path_so_far}': {e}")
            })?;
        }

        // Get the array
        let current_type = json_type_name(current);
        let arr = if let Some(obj) = current.as_object_mut() {
            if !obj.contains_key(field) {
                let available = format_available_keys(obj);
                return Err(crate::mp_err!(
                    "Field '{field}' not found. Available keys: {available}"
                ));
            }
            obj.get_mut(field).unwrap()
        } else {
            return Err(crate::mp_err!(
                "Cannot access '{field}': current value is {current_type} (expected object)"
            ));
        };

        let arr_type = json_type_name(arr);
        let arr = arr
            .as_array_mut()
            .ok_or_else(|| crate::mp_err!("Field '{field}' is {arr_type} (expected array)"))?;

        // Build remaining path after the selector
        let remaining_parts: Vec<&str> = parts.iter().skip(i + 1).copied().collect();

        if remaining_parts.is_empty() {
            // Selector is the last segment - replace array elements at indices
            for idx in indices {
                if idx < arr.len() {
                    arr[idx] = value.clone();
                }
            }
        } else {
            // Apply remaining path to selected array elements
            let remaining_path = remaining_parts.join(".");
            for idx in indices {
                if idx < arr.len() {
                    apply_jsonpath_patch(&mut arr[idx], &remaining_path, value.clone())?;
                }
            }
        }
        return Ok(());
    }

    // No multi-element selector found - use simple navigation
    let mut current = json;
    for (i, segment) in segments.iter().enumerate() {
        // Safety check: multi-element selectors should have been handled above
        if is_multi_element_selector(segment) {
            return Err(crate::mp_err!(
                "Internal error: multi-element selector was not handled. This is a bug."
            ));
        }

        if i == segments.len() - 1 {
            // Last segment: set the value
            match segment {
                PathSegment::Field(field) => {
                    if let Some(obj) = current.as_object_mut() {
                        obj.insert(field.to_string(), value);
                        return Ok(());
                    }
                    return Err(crate::mp_err!(
                        "Cannot set field '{}': current value is {} (expected object)",
                        field,
                        json_type_name(current)
                    ));
                }
                PathSegment::ArrayIndex(field, index) => {
                    if let Some(obj) = current.as_object_mut() {
                        let available = format_available_keys(obj);
                        let arr = obj.get_mut(*field).ok_or_else(|| {
                            crate::mp_err!("Field '{field}' not found. Available keys: {available}")
                        })?;
                        if let Some(arr_mut) = arr.as_array_mut() {
                            if *index < arr_mut.len() {
                                arr_mut[*index] = value;
                                return Ok(());
                            }
                            return Err(crate::mp_err!(
                                "Array index {} out of bounds for '{}' (length: {})",
                                index,
                                field,
                                arr_mut.len()
                            ));
                        }
                        return Err(crate::mp_err!(
                            "Field '{}' is {} (expected array)",
                            field,
                            json_type_name(arr)
                        ));
                    }
                    return Err(crate::mp_err!(
                        "Cannot access '{}[{}]': current value is {} (expected object)",
                        field,
                        index,
                        json_type_name(current)
                    ));
                }
                // Multi-element selectors are caught by the check above
                PathSegment::ArrayWildcard(_)
                | PathSegment::ArrayMultiIndex(_, _)
                | PathSegment::ArrayRange(_, _, _, _) => {
                    return Err(crate::mp_err!(
                        "Internal error: multi-element selector in last segment. This is a bug."
                    ));
                }
            }
        }
        // Intermediate segment: navigate deeper
        current = navigate_segment(current, segment, true).map_err(|e| {
            let path_so_far = parts[..=i].join(".");
            crate::mp_err!("At '{path_so_far}': {e}")
        })?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use http::{Response, StatusCode};

    #[tokio::test]
    async fn test_json_patch_add() {
        let json_body = r#"{"name": "John", "age": 30}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patch_str = r#"[
            {"op": "add", "path": "/email", "value": "john@example.com"}
        ]"#;
        let patch: json_patch::Patch = serde_json::from_str(patch_str).unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPatch(patch)]);
        let patched = patcher.apply(response, None).unwrap();

        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["name"], "John");
        assert_eq!(json["age"], 30);
        assert_eq!(json["email"], "john@example.com");
    }

    #[tokio::test]
    async fn test_json_patch_replace() {
        let json_body = r#"{"name": "John", "age": 30}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patch_str = r#"[
            {"op": "replace", "path": "/name", "value": "Jane"}
        ]"#;
        let patch: json_patch::Patch = serde_json::from_str(patch_str).unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPatch(patch)]);
        let patched = patcher.apply(response, None).unwrap();

        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["name"], "Jane");
        assert_eq!(json["age"], 30);
    }

    #[tokio::test]
    async fn test_json_patch_remove() {
        let json_body = r#"{"name": "John", "age": 30, "email": "john@example.com"}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patch_str = r#"[
            {"op": "remove", "path": "/email"}
        ]"#;
        let patch: json_patch::Patch = serde_json::from_str(patch_str).unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPatch(patch)]);
        let patched = patcher.apply(response, None).unwrap();

        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["name"], "John");
        assert_eq!(json["age"], 30);
        assert!(json.get("email").is_none());
    }

    #[tokio::test]
    async fn test_jsonpath_patch_simple() {
        let json_body = r#"{"user": {"name": "John"}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.user.email".to_string(),
            value: serde_json::json!("john@example.com"),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["user"]["email"], "john@example.com");
    }

    #[tokio::test]
    async fn test_jsonpath_patch_nested() {
        let json_body = r#"{"data": {"user": {"name": "John"}}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.data.user.age".to_string(),
            value: serde_json::json!(30),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["data"]["user"]["age"], 30);
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_index() {
        let json_body = r#"{"doc": {"signers": [{"name": "Alice", "language": "en"}, {"name": "Bob", "language": "en"}]}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![
            PatchOperation::JsonPath {
                path: "$.doc.signers[0].language".to_string(),
                value: serde_json::json!("ja"),
            },
            PatchOperation::JsonPath {
                path: "$.doc.signers[1].language".to_string(),
                value: serde_json::json!("fr"),
            },
        ]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["doc"]["signers"][0]["language"], "ja");
        assert_eq!(json["doc"]["signers"][1]["language"], "fr");
        // Ensure other fields are preserved
        assert_eq!(json["doc"]["signers"][0]["name"], "Alice");
        assert_eq!(json["doc"]["signers"][1]["name"], "Bob");
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_nested_field() {
        let json_body = r#"{"items": [{"id": 1, "meta": {"active": false}}, {"id": 2, "meta": {"active": false}}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.items[0].meta.active".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["items"][0]["meta"]["active"], true);
        assert_eq!(json["items"][1]["meta"]["active"], false); // unchanged
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_wildcard() {
        let json_body = r#"{"signers": [{"name": "Alice", "language": "en"}, {"name": "Bob", "language": "en"}, {"name": "Charlie", "language": "en"}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.signers[*].language".to_string(),
            value: serde_json::json!("ja"),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // All signers should have language changed to "ja"
        assert_eq!(json["signers"][0]["language"], "ja");
        assert_eq!(json["signers"][1]["language"], "ja");
        assert_eq!(json["signers"][2]["language"], "ja");
        // Names should be preserved
        assert_eq!(json["signers"][0]["name"], "Alice");
        assert_eq!(json["signers"][1]["name"], "Bob");
        assert_eq!(json["signers"][2]["name"], "Charlie");
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_wildcard_nested() {
        let json_body =
            r#"{"doc": {"signers": [{"meta": {"active": false}}, {"meta": {"active": false}}]}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.doc.signers[*].meta.active".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["doc"]["signers"][0]["meta"]["active"], true);
        assert_eq!(json["doc"]["signers"][1]["meta"]["active"], true);
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_wildcard_add_field() {
        let json_body = r#"{"signers": [{"name": "Alice"}, {"name": "Bob"}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.signers[*].force_language".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // New field should be added to all signers
        assert_eq!(json["signers"][0]["force_language"], true);
        assert_eq!(json["signers"][1]["force_language"], true);
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_multi_index() {
        let json_body = r#"{"items": [{"v": 0}, {"v": 1}, {"v": 2}, {"v": 3}, {"v": 4}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.items[0,2,4].active".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Only indices 0, 2, 4 should have "active" set
        assert_eq!(json["items"][0]["active"], true);
        assert!(json["items"][1].get("active").is_none());
        assert_eq!(json["items"][2]["active"], true);
        assert!(json["items"][3].get("active").is_none());
        assert_eq!(json["items"][4]["active"], true);
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_range_exclusive() {
        let json_body = r#"{"items": [{"v": 0}, {"v": 1}, {"v": 2}, {"v": 3}, {"v": 4}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.items[1..3].selected".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Range 1..3 is exclusive, so indices 1, 2 (not 3)
        assert!(json["items"][0].get("selected").is_none());
        assert_eq!(json["items"][1]["selected"], true);
        assert_eq!(json["items"][2]["selected"], true);
        assert!(json["items"][3].get("selected").is_none());
        assert!(json["items"][4].get("selected").is_none());
    }

    #[tokio::test]
    async fn test_jsonpath_patch_array_range_inclusive() {
        let json_body = r#"{"items": [{"v": 0}, {"v": 1}, {"v": 2}, {"v": 3}, {"v": 4}]}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.items[1..=3].selected".to_string(),
            value: serde_json::json!(true),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Range 1..=3 is inclusive, so indices 1, 2, 3
        assert!(json["items"][0].get("selected").is_none());
        assert_eq!(json["items"][1]["selected"], true);
        assert_eq!(json["items"][2]["selected"], true);
        assert_eq!(json["items"][3]["selected"], true);
        assert!(json["items"][4].get("selected").is_none());
    }

    #[tokio::test]
    async fn test_regex_replace() {
        let body_str = "Hello John, welcome to the system!";
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(body_str))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::RegexReplace {
            pattern: regex::Regex::new(r"\bJohn\b").unwrap(),
            replacement: "Jane".to_string(),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.into_body();
        let result = String::from_utf8(body.to_vec()).unwrap();

        assert_eq!(result, "Hello Jane, welcome to the system!");
    }

    #[tokio::test]
    async fn test_header_add() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-custom-header".to_string(),
            value: "test-value".to_string(),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let headers = patched.headers();

        assert_eq!(
            headers.get("x-custom-header").unwrap(),
            HeaderValue::from_static("test-value")
        );
    }

    #[tokio::test]
    async fn test_header_remove() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("x-to-remove", "value")
            .header("x-keep", "value")
            .body(Bytes::from("test"))
            .unwrap();

        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderRemove {
            name: "x-to-remove".to_string(),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        let headers = patched.headers();

        assert!(headers.get("x-to-remove").is_none());
        assert!(headers.get("x-keep").is_some());
    }

    #[tokio::test]
    async fn test_multiple_operations() {
        let json_body = r#"{"name": "John", "age": 30}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let patch_str = r#"[
            {"op": "add", "path": "/email", "value": "john@example.com"}
        ]"#;
        let patch: json_patch::Patch = serde_json::from_str(patch_str).unwrap();

        let patcher = ResponsePatcher::new(vec![
            PatchOperation::JsonPatch(patch),
            PatchOperation::JsonPath {
                path: "$.verified".to_string(),
                value: serde_json::json!(true),
            },
            PatchOperation::HeaderAdd {
                name: "x-patched".to_string(),
                value: "true".to_string(),
            },
        ]);

        let patched = patcher.apply(response, None).unwrap();
        let body = patched.body();
        let json: serde_json::Value = serde_json::from_slice(body).unwrap();

        assert_eq!(json["email"], "john@example.com");
        assert_eq!(json["verified"], true);
        assert_eq!(
            patched.headers().get("x-patched").unwrap(),
            HeaderValue::from_static("true")
        );
    }

    #[test]
    fn test_jsonpath_error_shows_available_keys_at_correct_level() {
        // Test case: path $.signrequest.signers[*].language where signers doesn't exist inside signrequest
        let json_body =
            r#"{"signrequest": {"id": "123", "status": "pending"}, "doc": {"name": "test.pdf"}}"#;

        let mut json: serde_json::Value = serde_json::from_str(json_body).unwrap();
        let result = apply_jsonpath_patch(
            &mut json,
            "$.signrequest.signers[*].language",
            serde_json::json!("ja"),
        );

        // Should fail and show keys available inside signrequest, not at root
        let err = result.unwrap_err();
        let err_msg = err.to_string();

        // Error should mention 'signers' is not found
        assert!(
            err_msg.contains("signers"),
            "Error should mention 'signers': {err_msg}"
        );

        // Error should show available keys at the signrequest level (id, status), not root level (doc, signrequest)
        assert!(
            err_msg.contains("id") && err_msg.contains("status"),
            "Error should show keys inside signrequest (id, status): {err_msg}"
        );

        // Error should NOT show root-level keys (that was the bug)
        assert!(
            !err_msg.contains("doc"),
            "Error should NOT show root-level keys like 'doc': {err_msg}"
        );
    }

    #[test]
    fn test_jsonpath_error_shows_type_mismatch() {
        // Test case: trying to access array on a string field
        let json_body = r#"{"user": {"name": "John"}}"#;

        let mut json: serde_json::Value = serde_json::from_str(json_body).unwrap();
        let result =
            apply_jsonpath_patch(&mut json, "$.user.name[0].foo", serde_json::json!("bar"));

        let err = result.unwrap_err();
        let err_msg = err.to_string();

        // Error should mention that name is a string, not an array/object
        assert!(
            err_msg.contains("string"),
            "Error should indicate the actual type: {err_msg}"
        );
    }

    // ==================== Template rendering tests ====================

    fn make_patch_context() -> crate::types::PatchContext {
        let mut captures = rustc_hash::FxHashMap::default();
        captures.insert("id".to_string(), "42".to_string());
        captures.insert("name".to_string(), "test-user".to_string());

        let mut headers = rustc_hash::FxHashMap::default();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let mut response_headers = rustc_hash::FxHashMap::default();
        response_headers.insert("x-request-id".to_string(), "req-abc-123".to_string());

        crate::types::PatchContext {
            request: crate::types::RequestContext {
                method: "GET".to_string(),
                uri: "/api/users/42".to_string(),
                path: "/api/users/42".to_string(),
                captures,
                headers,
                query: rustc_hash::FxHashMap::default(),
                body: None,
                body_bytes: None,
                body_json: None,
                vars: None,
            },
            response_status: 200,
            response_headers,
            response_body_json: Some(serde_json::json!({"user": {"name": "Alice"}, "count": 5})),
        }
    }

    #[tokio::test]
    async fn test_template_jsonpath_value_with_captures() {
        let json_body = r#"{"user": {"name": "placeholder"}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.user.id".to_string(),
            // "42" renders and re-parses as a JSON number since it's valid JSON
            value: serde_json::json!("{{ captures.id }}"),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // captures.id is "42" which parses as number 42
        assert_eq!(json["user"]["id"], 42);
    }

    #[tokio::test]
    async fn test_template_jsonpath_string_value_with_captures() {
        let json_body = r#"{"user": {"name": "placeholder"}}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.user.display_name".to_string(),
            // Non-numeric string stays as string
            value: serde_json::json!("user-{{ captures.name }}"),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["user"]["display_name"], "user-test-user");
    }

    #[tokio::test]
    async fn test_template_jsonpath_producing_number() {
        let json_body = r#"{"count": 0}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let ctx = make_patch_context();
        // response.body_json.count is 5
        let patcher = ResponsePatcher::new(vec![PatchOperation::JsonPath {
            path: "$.count".to_string(),
            value: serde_json::json!("{{ response.body_json.count }}"),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let body = patched.into_body();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Template rendered "5", which gets parsed as a JSON number
        assert_eq!(json["count"], 5);
    }

    #[tokio::test]
    async fn test_template_header_add_value() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-user-id".to_string(),
            value: "{{ captures.id }}".to_string(),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        assert_eq!(patched.headers().get("x-user-id").unwrap(), "42");
    }

    #[tokio::test]
    async fn test_template_header_add_response_status() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-upstream-status".to_string(),
            value: "{{ response.status }}".to_string(),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        assert_eq!(patched.headers().get("x-upstream-status").unwrap(), "200");
    }

    #[tokio::test]
    async fn test_template_regex_replacement() {
        let body_str = "Hello USER_NAME, your ID is USER_ID.";
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(body_str))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![
            PatchOperation::RegexReplace {
                pattern: regex::Regex::new(r"\bUSER_NAME\b").unwrap(),
                replacement: "{{ captures.name }}".to_string(),
            },
            PatchOperation::RegexReplace {
                pattern: regex::Regex::new(r"\bUSER_ID\b").unwrap(),
                replacement: "{{ captures.id }}".to_string(),
            },
        ]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let body = patched.into_body();
        let result = String::from_utf8(body.to_vec()).unwrap();

        assert_eq!(result, "Hello test-user, your ID is 42.");
    }

    #[tokio::test]
    async fn test_literal_values_pass_through_unchanged() {
        let json_body = r#"{"name": "John"}"#;
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from(json_body))
            .unwrap();

        let ctx = make_patch_context();
        // No template markers, so values should be used literally
        let patcher = ResponsePatcher::new(vec![
            PatchOperation::JsonPath {
                path: "$.email".to_string(),
                value: serde_json::json!("john@example.com"),
            },
            PatchOperation::HeaderAdd {
                name: "x-static".to_string(),
                value: "plain-value".to_string(),
            },
        ]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let body = patched.body();
        let json: serde_json::Value = serde_json::from_slice(body).unwrap();

        assert_eq!(json["email"], "john@example.com");
        assert_eq!(patched.headers().get("x-static").unwrap(), "plain-value");
    }

    #[tokio::test]
    async fn test_template_render_error_falls_back_to_literal() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        let ctx = make_patch_context();
        // Invalid template syntax - should fall back to literal
        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-test".to_string(),
            value: "{{ nonexistent_var }}".to_string(),
        }]);

        // Should not error - falls back to the literal string
        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        let header_val = patched.headers().get("x-test").unwrap().to_str().unwrap();
        // Falls back to either the literal template string or empty string
        // depending on Tera's behavior - the key point is it doesn't error
        assert!(!header_val.is_empty());
    }

    #[tokio::test]
    async fn test_no_context_means_no_rendering() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        // Pass None for context - templates should be treated as literals
        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-test".to_string(),
            value: "{{ captures.id }}".to_string(),
        }]);

        let patched = patcher.apply(response, None).unwrap();
        // Without context, the template string is used as-is
        assert_eq!(
            patched.headers().get("x-test").unwrap(),
            "{{ captures.id }}"
        );
    }

    #[tokio::test]
    async fn test_template_response_headers_access() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .body(Bytes::from("test"))
            .unwrap();

        let ctx = make_patch_context();
        let patcher = ResponsePatcher::new(vec![PatchOperation::HeaderAdd {
            name: "x-echo-request-id".to_string(),
            value: "{{ response.headers['x-request-id'] }}".to_string(),
        }]);

        let patched = patcher.apply(response, Some(&ctx)).unwrap();
        assert_eq!(
            patched.headers().get("x-echo-request-id").unwrap(),
            "req-abc-123"
        );
    }
}
