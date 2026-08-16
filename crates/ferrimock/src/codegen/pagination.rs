//! Pagination template generation
//!
//! This module handles generation of pagination-related templates including
//! preambles, field generation, and URL construction for offset, cursor, and page-based pagination.

use super::types::{PaginationInfo, PaginationType};
use rustc_hash::FxHashSet;
use std::fmt::Write;

/// What separates the page number inside a generated cursor.
///
/// A marker is opaque to the client -- it hands back whatever it was given --
/// so the page it stands for is written into it. That is what lets the next
/// answer be decided by the request instead of by state on the server.
const CURSOR_MARK: &str = "_";

/// Generate preamble for offset-based pagination
pub(super) fn generate_offset_pagination_preamble(
    pagination: &PaginationInfo,
    storage_path: &str,
    pagination_storage_key_template: &str,
) -> String {
    let mut preamble = String::new();

    // Convert query params to int for math operations
    if let Some(ref limit_field) = pagination.limit_field {
        let _ = writeln!(
            preamble,
            "{{%- set limit = query.{limit_field} | default(value=\"20\") | int -%}}"
        );
    } else {
        preamble.push_str("{%- set limit = query.limit | default(value=\"20\") | int -%}\n");
    }

    if let Some(ref offset_field) = pagination.offset_field {
        let _ = writeln!(
            preamble,
            "{{%- set offset = query.{offset_field} | default(value=\"0\") | int -%}}"
        );
    } else {
        preamble.push_str("{%- set offset = query.offset | default(value=\"0\") | int -%}\n");
    }

    if let Some(ref _total_field) = pagination.total_field {
        let default_total = pagination.sample_total.unwrap_or(100);
        let storage_key = pagination_storage_key_template.replace("{path}", storage_path);
        let _ = writeln!(
            preamble,
            "{{%- set total = store_get_or_set(key=\"{storage_key}\", default={default_total}) -%}}"
        );
    }

    if pagination.has_more_field.is_some() {
        if pagination.total_field.is_some() {
            // If we have a total field, use it for has_more calculation
            preamble.push_str("{%- set has_more = (offset + limit) < total -%}\n");
        } else {
            // Without a total field, use a reasonable max offset (10,000 items)
            preamble.push_str("{%- set has_more = offset < 10000 -%}\n");
        }
    }

    preamble
}

/// Generate preamble for cursor-based pagination
pub(super) fn generate_cursor_pagination_preamble(
    pagination: &PaginationInfo,
    storage_path: &str,
    pagination_storage_key_template: &str,
) -> String {
    let mut preamble = String::new();

    // Always define limit - either from query param or with default
    // Convert to int for math operations
    if let Some(ref limit_field) = pagination.limit_field {
        let _ = writeln!(
            preamble,
            "{{%- set limit = query.{limit_field} | default(value=\"20\") | int -%}}"
        );
    } else {
        // Even if no limit_field detected, define limit with default for templates that use it
        preamble.push_str("{%- set limit = query.limit | default(value=\"20\") | int -%}\n");
    }

    if let Some(ref _total_field) = pagination.total_field {
        let default_total = pagination.sample_total.unwrap_or(100);
        let storage_key = pagination_storage_key_template.replace("{path}", storage_path);
        let _ = writeln!(
            preamble,
            "{{%- set total = store_get_or_set(key=\"{storage_key}\", default={default_total}) -%}}"
        );
    }

    // Which page the client asked for is written in the marker it was handed,
    // so the answer is decided by the request rather than by a counter on the
    // server. A counter makes the same request answer differently each time it
    // is made, which no cursor endpoint does, and leaves a client that retries
    // one page reading the next.
    if let Some(cursor_param) = pagination.cursor_param.as_deref() {
        let _ = writeln!(
            preamble,
            "{{%- set cursor = query.{cursor_param} | default(value=\"\") | split(pat=\"{CURSOR_MARK}\") -%}}"
        );
        // The prefix has to be checked before the number is read. A real
        // cursor is opaque and may carry the separator anywhere in it, and
        // asking Tera to read a chunk of base64 as an integer fails the whole render.
        preamble.push_str(
            "{%- set page_num = cursor | length > 2 and cursor[0] == \"page\" \
                 and cursor[1] | int(default=1) or 1 -%}\n",
        );
    } else {
        // No recording ever showed the parameter, so there is nothing in the
        // request to read and a counter is all that is left.
        let page_key = format!("{storage_path}.cursor.page");
        let _ = writeln!(
            preamble,
            "{{%- set page_num = store_incr(key=\"{page_key}\") -%}}"
        );
    }

    if pagination.has_more_field.is_some() || pagination.next_field.is_some() {
        if pagination.total_field.is_some() {
            // If we have a total field, use it for has_more calculation
            preamble.push_str("{%- set has_more = (page_num * limit) < total -%}\n");
        } else {
            // Without a total field, use a reasonable max page limit to prevent infinite pagination
            // Limit to 10 pages for development/testing (can be increased if needed)
            preamble.push_str("{%- set has_more = page_num < 10 -%}\n");
        }
    }

    preamble
}

/// Generate preamble for page-based pagination
pub(super) fn generate_page_pagination_preamble(
    pagination: &PaginationInfo,
    storage_path: &str,
    pagination_storage_key_template: &str,
) -> String {
    let mut preamble = String::new();

    // Always define limit - either from query param or with default
    // Convert to int for math operations
    if let Some(ref limit_field) = pagination.limit_field {
        let _ = writeln!(
            preamble,
            "{{%- set limit = query.{limit_field} | default(value=\"20\") | int -%}}"
        );
    } else {
        // Even if no limit_field detected, define limit with default for templates that use it
        preamble.push_str("{%- set limit = query.limit | default(value=\"20\") | int -%}\n");
    }

    preamble.push_str("{%- set page = query.page | default(value=\"1\") | int -%}\n");

    if let Some(ref _total_field) = pagination.total_field {
        let default_total = pagination.sample_total.unwrap_or(100);
        let storage_key = pagination_storage_key_template.replace("{path}", storage_path);
        let _ = writeln!(
            preamble,
            "{{%- set total = store_get_or_set(key=\"{storage_key}\", default={default_total}) -%}}"
        );
        preamble
            .push_str("{%- set total_pages = (total / limit) | round(method=\"ceil\") | int -%}\n");
        preamble.push_str("{%- set has_more = page < total_pages -%}\n");
    } else {
        // Without a total field, use a reasonable max page limit to prevent infinite pagination
        // Limit to 10 pages for development/testing (can be increased if needed)
        preamble.push_str("{%- set has_more = page < 10 -%}\n");
    }

    preamble
}

/// Generate pagination field templates
pub(super) fn generate_pagination_fields(
    pagination: &PaginationInfo,
    pagination_fields: &mut FxHashSet<String>,
) -> Vec<String> {
    let mut fields = Vec::new();

    if let Some(ref total_field) = pagination.total_field {
        pagination_fields.insert(total_field.clone());
        fields.push(format!("  \"{total_field}\": {{{{ total }}}}"));
    }

    if let Some(ref offset_field) = pagination.offset_field {
        pagination_fields.insert(offset_field.clone());
        if pagination.pagination_type == PaginationType::Offset {
            fields.push(format!("  \"{offset_field}\": {{{{ offset }}}}"));
        } else if pagination.pagination_type == PaginationType::Cursor {
            fields.push(format!(
                "  \"{offset_field}\": {{{{ (page_num - 1) * limit }}}}"
            ));
        }
    }

    if let Some(ref limit_field) = pagination.limit_field {
        pagination_fields.insert(limit_field.clone());
        fields.push(format!("  \"{limit_field}\": {{{{ limit }}}}"));
    }

    if let Some(ref has_more_field) = pagination.has_more_field {
        pagination_fields.insert(has_more_field.clone());
        fields.push(format!("  \"{has_more_field}\": {{{{ has_more }}}}"));
    }

    if let Some(ref next_field) = pagination.next_field {
        pagination_fields.insert(next_field.clone());
        fields.push(generate_next_field(
            next_field,
            &pagination.pagination_type,
            pagination.link_base.as_deref(),
            &pagination.static_query_params,
        ));
    }

    if let Some(ref prev_field) = pagination.prev_field {
        pagination_fields.insert(prev_field.clone());
        fields.push(generate_prev_field(
            prev_field,
            &pagination.pagination_type,
            pagination.link_base.as_deref(),
            &pagination.static_query_params,
        ));
    }

    fields
}

/// Where a pagination link points.
///
/// The recording's own links are the ground truth: they say which host and path
/// the client is expected to come back to. Without one -- the first page has no
/// `previous`, and cursor pagination carries no URL at all -- the request's own
/// path is the closest thing to right, and a link back to the same mock is
/// worth more than an absolute one pointing nowhere. Inventing a host is not an
/// option: it is drawn again on every render, so `next` and `previous` in the
/// same answer would disagree about which endpoint they belong to.
fn link_target(link_base: Option<&str>) -> String {
    link_base.map_or_else(|| "{{ path }}".to_string(), ToString::to_string)
}

/// Generate next field template
pub(super) fn generate_next_field(
    next_field: &str,
    pagination_type: &PaginationType,
    link_base: Option<&str>,
    static_params: &str,
) -> String {
    let base_path = link_target(link_base);
    let param_sep = if static_params.is_empty() { "" } else { "&" };

    match pagination_type {
        PaginationType::Offset => {
            if static_params.is_empty() {
                format!(
                    "  \"{next_field}\": {{% if has_more %}}\"{base_path}?offset={{{{ offset + limit }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            } else {
                format!(
                    "  \"{next_field}\": {{% if has_more %}}\"{base_path}?{static_params}&offset={{{{ offset + limit }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            }
        }
        PaginationType::Cursor => {
            format!(
                "  \"{next_field}\": {{% if has_more %}}\"page{CURSOR_MARK}{{{{ page_num + 1 }}}}{CURSOR_MARK}{{{{ fake_token() }}}}\"{{% else %}}null{{% endif %}}"
            )
        }
        PaginationType::Page => {
            if static_params.is_empty() {
                format!(
                    "  \"{next_field}\": {{% if has_more %}}\"{base_path}?page={{{{ page + 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            } else {
                format!(
                    "  \"{next_field}\": {{% if has_more %}}\"{base_path}?{static_params}{param_sep}page={{{{ page + 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            }
        }
    }
}

/// Generate previous field template
pub(super) fn generate_prev_field(
    prev_field: &str,
    pagination_type: &PaginationType,
    link_base: Option<&str>,
    static_params: &str,
) -> String {
    let base_path = link_target(link_base);
    let param_sep = if static_params.is_empty() { "" } else { "&" };

    match pagination_type {
        PaginationType::Offset => {
            if static_params.is_empty() {
                format!(
                    "  \"{prev_field}\": {{% if offset > 0 %}}\"{base_path}?offset={{% if offset > limit %}}{{{{ offset - limit }}}}{{% else %}}0{{% endif %}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            } else {
                format!(
                    "  \"{prev_field}\": {{% if offset > 0 %}}\"{base_path}?{static_params}&offset={{% if offset > limit %}}{{{{ offset - limit }}}}{{% else %}}0{{% endif %}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            }
        }
        PaginationType::Cursor => {
            format!(
                "  \"{prev_field}\": {{% if page_num > 1 %}}\"page{CURSOR_MARK}{{{{ page_num - 1 }}}}{CURSOR_MARK}{{{{ fake_token() }}}}\"{{% else %}}null{{% endif %}}"
            )
        }
        PaginationType::Page => {
            if static_params.is_empty() {
                format!(
                    "  \"{prev_field}\": {{% if page > 1 %}}\"{base_path}?page={{{{ page - 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            } else {
                format!(
                    "  \"{prev_field}\": {{% if page > 1 %}}\"{base_path}?{static_params}{param_sep}page={{{{ page - 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
                )
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn cursor_info(cursor_param: Option<&str>) -> PaginationInfo {
        PaginationInfo {
            total_field: None,
            offset_field: None,
            limit_field: Some("limit".to_string()),
            next_field: Some("next_marker".to_string()),
            prev_field: None,
            has_more_field: None,
            sample_total: None,
            pagination_type: PaginationType::Cursor,
            static_query_params: String::new(),
            link_base: None,
            cursor_param: cursor_param.map(ToString::to_string),
        }
    }

    #[test]
    fn a_cursor_endpoint_answers_the_marker_it_was_given() {
        let preamble = generate_cursor_pagination_preamble(
            &cursor_info(Some("marker")),
            "api.docs",
            "api.{path}.total",
        );

        assert!(
            preamble.contains("query.marker"),
            "the page asked for is in the request: {preamble}"
        );
        assert!(
            !preamble.contains("store_incr"),
            "a counter makes the same request answer differently each time: {preamble}"
        );
        // A real marker is opaque and carries separators of its own; reading a
        // page number out of one without checking the prefix fails the render.
        assert!(
            preamble.contains("cursor[0] == \"page\""),
            "the marker's own prefix has to be checked first: {preamble}"
        );
    }

    #[test]
    fn a_cursor_endpoint_that_never_showed_its_parameter_falls_back_to_counting() {
        let preamble =
            generate_cursor_pagination_preamble(&cursor_info(None), "api.docs", "api.{path}.total");

        assert!(preamble.contains("store_incr"));
    }

    #[test]
    fn a_pagination_link_points_where_the_recording_pointed() {
        let next = generate_next_field(
            "next",
            &PaginationType::Page,
            Some("https://api.example.com/v2/items"),
            "",
        );
        assert!(
            next.contains("https://api.example.com/v2/items?page="),
            "{next}"
        );

        // With nothing recorded, the request's own path leads back here. A
        // freshly invented host is drawn again on every render, so `next` and
        // `previous` in one answer would disagree.
        let unknown = generate_next_field("next", &PaginationType::Page, None, "");
        assert!(unknown.contains("{{ path }}?page="), "{unknown}");
    }
}
