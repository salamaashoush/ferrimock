//! Pagination template generation
//!
//! This module handles generation of pagination-related templates including
//! preambles, field generation, and URL construction for offset, cursor, and page-based pagination.

use crate::types::{PaginationInfo, PaginationType};
use rustc_hash::FxHashSet;
use std::fmt::Write;

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

  let page_key = format!("{storage_path}.cursor.page");
  let _ = writeln!(preamble, "{{%- set page_num = store_incr(key=\"{page_key}\") -%}}");

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
    preamble.push_str("{%- set total_pages = (total / limit) | round(method=\"ceil\") | int -%}\n");
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
  base_path: &str,
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
      fields.push(format!("  \"{offset_field}\": {{{{ (page_num - 1) * limit }}}}"));
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
      base_path,
      &pagination.static_query_params,
    ));
  }

  if let Some(ref prev_field) = pagination.prev_field {
    pagination_fields.insert(prev_field.clone());
    fields.push(generate_prev_field(
      prev_field,
      &pagination.pagination_type,
      base_path,
      &pagination.static_query_params,
    ));
  }

  fields
}

/// Generate next field template
pub(super) fn generate_next_field(
  next_field: &str,
  pagination_type: &PaginationType,
  base_path: &str,
  static_params: &str,
) -> String {
  let param_sep = if static_params.is_empty() { "" } else { "&" };

  match pagination_type {
    PaginationType::Offset => {
      if static_params.is_empty() {
        format!(
          "  \"{next_field}\": {{% if has_more %}}\"{{{{ fake_api_url() }}}}{base_path}?offset={{{{ offset + limit }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      } else {
        format!(
          "  \"{next_field}\": {{% if has_more %}}\"{{{{ fake_api_url() }}}}{base_path}?{static_params}&offset={{{{ offset + limit }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      }
    },
    PaginationType::Cursor => {
      format!(
        "  \"{next_field}\": {{% if has_more %}}\"cursor_page_{{{{ page_num + 1 }}}}_{{{{ uuid() | truncate(length=8, end=\"\") }}}}\"{{% else %}}null{{% endif %}}"
      )
    },
    PaginationType::Page => {
      if static_params.is_empty() {
        format!(
          "  \"{next_field}\": {{% if has_more %}}\"{{{{ fake_api_url() }}}}{base_path}?page={{{{ page + 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      } else {
        format!(
          "  \"{next_field}\": {{% if has_more %}}\"{{{{ fake_api_url() }}}}{base_path}?{static_params}{param_sep}page={{{{ page + 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      }
    },
  }
}

/// Generate previous field template
pub(super) fn generate_prev_field(
  prev_field: &str,
  pagination_type: &PaginationType,
  base_path: &str,
  static_params: &str,
) -> String {
  let param_sep = if static_params.is_empty() { "" } else { "&" };

  match pagination_type {
    PaginationType::Offset => {
      if static_params.is_empty() {
        format!(
          "  \"{prev_field}\": {{% if offset > 0 %}}\"{{{{ fake_api_url() }}}}{base_path}?offset={{{{ [0, offset - limit] | max }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      } else {
        format!(
          "  \"{prev_field}\": {{% if offset > 0 %}}\"{{{{ fake_api_url() }}}}{base_path}?{static_params}&offset={{{{ [0, offset - limit] | max }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      }
    },
    PaginationType::Cursor => {
      format!(
        "  \"{prev_field}\": {{% if page_num > 1 %}}\"cursor_page_{{{{ page_num - 1 }}}}_{{{{ uuid() | truncate(length=8, end=\"\") }}}}\"{{% else %}}null{{% endif %}}"
      )
    },
    PaginationType::Page => {
      if static_params.is_empty() {
        format!(
          "  \"{prev_field}\": {{% if page > 1 %}}\"{{{{ fake_api_url() }}}}{base_path}?page={{{{ page - 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      } else {
        format!(
          "  \"{prev_field}\": {{% if page > 1 %}}\"{{{{ fake_api_url() }}}}{base_path}?{static_params}{param_sep}page={{{{ page - 1 }}}}&limit={{{{ limit }}}}\"{{% else %}}null{{% endif %}}"
        )
      }
    },
  }
}
