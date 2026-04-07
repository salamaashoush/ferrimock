//! Main template generator
//!
//! This module contains the TemplateGenerator struct and the main template generation logic.

use super::types::{PaginationType, ResponseStructure};
use crate::type_detector::FieldType;
use rustc_hash::FxHashSet;

use super::array_object::{generate_tera_array_with_limit, generate_tera_object_with_extension};
use super::field_converter::{field_type_to_tera_expr, field_type_to_tera_expr_with_context};
use super::file_detection::extract_file_extension_from_response;
use super::helpers::detect_results_array_field;
use super::pagination::{
    generate_cursor_pagination_preamble, generate_offset_pagination_preamble,
    generate_page_pagination_preamble, generate_pagination_fields,
};

/// Template generator for creating Tera templates from response analysis
pub struct TemplateGenerator {
    pagination_storage_key_template: String,
}

impl TemplateGenerator {
    pub fn new(pagination_storage_key_template: String) -> Self {
        Self {
            pagination_storage_key_template,
        }
    }

    /// Generate a Tera template from analysis
    pub fn generate_tera_template(
        &self,
        analysis: &ResponseStructure,
        base_path: &str,
        graphql_analysis: &crate::codegen::types::GraphQLVariableInfo,
    ) -> String {
        if !analysis.is_json {
            return "{}".to_string();
        }

        if analysis.top_level_type == "array" {
            return Self::generate_tera_template_for_array(analysis);
        }

        if analysis.top_level_type != "object" {
            return "{}".to_string();
        }

        let mut fields = Vec::new();
        let mut pagination_fields = FxHashSet::default();

        if let Some(ref pagination) = analysis.pagination {
            let storage_path = base_path
                .trim_start_matches('/')
                .replace('/', ".")
                .replace('-', "_");
            let mut preamble = String::new();

            match pagination.pagination_type {
                PaginationType::Offset => {
                    preamble.push_str(&generate_offset_pagination_preamble(
                        pagination,
                        &storage_path,
                        &self.pagination_storage_key_template,
                    ));
                }
                PaginationType::Cursor => {
                    preamble.push_str(&generate_cursor_pagination_preamble(
                        pagination,
                        &storage_path,
                        &self.pagination_storage_key_template,
                    ));
                }
                PaginationType::Page => {
                    preamble.push_str(&generate_page_pagination_preamble(
                        pagination,
                        &storage_path,
                        &self.pagination_storage_key_template,
                    ));
                }
            }

            // Generate pagination fields (count, next, previous, etc.)
            let pagination_field_strs =
                generate_pagination_fields(pagination, base_path, &mut pagination_fields);

            if !preamble.is_empty() {
                // Combine pagination fields with remaining fields
                let mut all_fields = pagination_field_strs;
                all_fields.extend(Self::generate_remaining_fields(
                    analysis,
                    &pagination_fields,
                    graphql_analysis,
                ));

                return format!("{}{{\n{}\n}}", preamble, all_fields.join(",\n"));
            }
        }

        // Check if this is a pagination results array field
        let results_array_field = detect_results_array_field(analysis);

        // Detect file extension for Box file objects
        let file_extension = extract_file_extension_from_response(analysis);

        for (field, field_type) in &analysis.varying_fields {
            if !pagination_fields.contains(field) {
                // Check if this field matches a GraphQL variable
                let graphql_var_expr =
                    Self::try_graphql_variable_expression(field, graphql_analysis);

                let expr = if let Some(gql_expr) = graphql_var_expr {
                    // Use GraphQL variable extraction
                    gql_expr
                } else if results_array_field.as_ref() == Some(field) {
                    // This is the pagination results array - use limit instead of random count
                    Self::field_type_to_tera_expr_with_limit(
                        field,
                        field_type,
                        analysis.has_matching_path_ids,
                    )
                } else if matches!(field_type, FieldType::DownloadUrl { .. })
                    && file_extension.is_some()
                {
                    // For download URLs in Box file objects, use file extension
                    field_type_to_tera_expr_with_context(
                        field,
                        field_type,
                        analysis.has_matching_path_ids,
                        file_extension.as_deref(),
                    )
                } else if matches!(field_type, FieldType::Url)
                    && file_extension.is_some()
                    && (field.contains("download_url") || field.contains("download"))
                {
                    // Treat URLs with "download" in name as download URLs in Box file objects
                    field_type_to_tera_expr_with_context(
                        field,
                        &FieldType::DownloadUrl { sample_url: None },
                        analysis.has_matching_path_ids,
                        file_extension.as_deref(),
                    )
                } else if matches!(field_type, FieldType::Object(_)) && file_extension.is_some() {
                    // For nested objects, pass extension context if available
                    Self::field_type_to_tera_expr_with_extension(
                        field,
                        field_type,
                        analysis.has_matching_path_ids,
                        file_extension.as_deref(),
                        graphql_analysis,
                    )
                } else if matches!(field_type, FieldType::Object(_)) {
                    // For nested objects without extension, still pass GraphQL analysis
                    Self::field_type_to_tera_expr_with_extension(
                        field,
                        field_type,
                        analysis.has_matching_path_ids,
                        None,
                        graphql_analysis,
                    )
                } else {
                    field_type_to_tera_expr(field, field_type, analysis.has_matching_path_ids)
                };
                fields.push(format!("  \"{field}\": {expr}"));
            }
        }

        for (field, value) in &analysis.constant_fields {
            if !pagination_fields.contains(field) {
                let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
                fields.push(format!("  \"{field}\": {value_str}"));
            }
        }

        if fields.is_empty() {
            return "{}".to_string();
        }

        format!("{{\n{}\n}}", fields.join(",\n"))
    }

    fn try_graphql_variable_expression(
        field: &str,
        graphql_analysis: &crate::codegen::types::GraphQLVariableInfo,
    ) -> Option<String> {
        if !graphql_analysis.has_varying_variables {
            return None;
        }

        // Check if this field matches a GraphQL variable name
        // We need to handle nested paths like "user.id" -> variables.id
        // or "data.user.id" -> variables.id

        // Common GraphQL response patterns:
        // - data.user.id -> check if "id" is a variable
        // - data.users[0].id -> check if "id" is a variable
        // - user.id -> check if "id" is a variable

        let field_parts: Vec<&str> = field.split('.').collect();

        // Try to match the last component (most common case: data.user.id -> id)
        if let Some(last_part) = field_parts.last() {
            // Remove array indices like [0] from the field name
            let clean_part = last_part.split('[').next().unwrap_or(last_part);

            if graphql_analysis
                .varying_variables
                .contains(&clean_part.to_string())
            {
                return Some(format!("{{{{ body_json.variables.{clean_part} }}}}"));
            }
        }

        // Try to match the full path if it contains only one component
        if field_parts.len() == 1 {
            let clean_field = field.split('[').next().unwrap_or(field);
            if graphql_analysis
                .varying_variables
                .contains(&clean_field.to_string())
            {
                return Some(format!("{{{{ body_json.variables.{clean_field} }}}}"));
            }
        }

        None
    }
}

/// Public helper to check GraphQL variable expression (for use in nested object processing)
pub(super) fn try_graphql_variable_expression_for_nested(
    field: &str,
    graphql_analysis: &crate::codegen::types::GraphQLVariableInfo,
) -> Option<String> {
    TemplateGenerator::try_graphql_variable_expression(field, graphql_analysis)
}

impl TemplateGenerator {
    fn generate_remaining_fields(
        analysis: &ResponseStructure,
        pagination_fields: &FxHashSet<String>,
        graphql_analysis: &crate::codegen::types::GraphQLVariableInfo,
    ) -> Vec<String> {
        let mut fields = Vec::new();

        // Detect if there's a pagination results array field
        let results_array_field = detect_results_array_field(analysis);

        for (field, field_type) in &analysis.varying_fields {
            if !pagination_fields.contains(field) {
                // Check if this field matches a GraphQL variable
                let graphql_var_expr =
                    Self::try_graphql_variable_expression(field, graphql_analysis);

                let expr = if let Some(gql_expr) = graphql_var_expr {
                    // Use GraphQL variable extraction
                    gql_expr
                } else if results_array_field.as_ref() == Some(field) {
                    // This is the pagination results array - use limit instead of random count
                    Self::field_type_to_tera_expr_with_limit(
                        field,
                        field_type,
                        analysis.has_matching_path_ids,
                    )
                } else {
                    field_type_to_tera_expr(field, field_type, analysis.has_matching_path_ids)
                };
                fields.push(format!("  \"{field}\": {expr}"));
            }
        }

        for (field, value) in &analysis.constant_fields {
            if !pagination_fields.contains(field) {
                let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
                fields.push(format!("  \"{field}\": {value_str}"));
            }
        }

        fields
    }

    fn generate_tera_template_for_array(analysis: &ResponseStructure) -> String {
        if analysis.varying_fields.is_empty() && analysis.constant_fields.is_empty() {
            return "[]".to_string();
        }

        let mut fields = Vec::new();

        for (field, field_type) in &analysis.varying_fields {
            let expr = field_type_to_tera_expr(field, field_type, false);
            fields.push(format!("    \"{field}\": {expr}"));
        }

        for (field, value) in &analysis.constant_fields {
            let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
            fields.push(format!("    \"{field}\": {value_str}"));
        }

        format!(
            "[\n  {{% for i in range(end=get_random(start=5, end=15)) %}}\n  {{\n{}\n  }}{{% if not loop.last %}},{{% endif %}}\n  {{% endfor %}}\n]",
            fields.join(",\n")
        )
    }

    /// Generate Tera expression for a field that should use `limit` for array size (pagination results)
    pub fn field_type_to_tera_expr_with_limit(
        field_name: &str,
        field_type: &FieldType,
        has_matching_path_ids: bool,
    ) -> String {
        match field_type {
            FieldType::Array(pattern) => generate_tera_array_with_limit(pattern),
            _ => field_type_to_tera_expr(field_name, field_type, has_matching_path_ids),
        }
    }

    /// Generate Tera expression for nested objects with extension context
    fn field_type_to_tera_expr_with_extension(
        field_name: &str,
        field_type: &FieldType,
        has_matching_path_ids: bool,
        extension: Option<&str>,
        graphql_analysis: &crate::codegen::types::GraphQLVariableInfo,
    ) -> String {
        match field_type {
            FieldType::Object(analysis) => generate_tera_object_with_extension(
                analysis,
                has_matching_path_ids,
                extension,
                graphql_analysis,
            ),
            _ => field_type_to_tera_expr(field_name, field_type, has_matching_path_ids),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::types::PaginationInfo;
    use super::*;
    use crate::type_detector::{ArrayPattern, FieldType};

    #[test]
    fn test_box_file_object_with_pdf_download_url() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        // Simulate a Box file object response with sign.download_url
        let sign_object = FieldType::Object(Box::new(crate::type_detector::ObjectAnalysis {
            varying_fields: vec![(
                "download_url".to_string(),
                FieldType::DownloadUrl {
                    sample_url: Some(
                        "https://dl.boxcloud.com/d/1/b0!6mFruo9M1-NhU-3a1Ou3mh...".to_string(),
                    ),
                },
            )],
            constant_fields: vec![(
                "download_status".to_string(),
                serde_json::Value::String("success".to_string()),
            )],
        }));

        let analysis = ResponseStructure {
            varying_fields: vec![
                ("id".to_string(), FieldType::NumericStringId),
                ("name".to_string(), FieldType::Name),
                ("sign".to_string(), sign_object),
                ("size".to_string(), FieldType::FileSize),
            ],
            constant_fields: vec![
                (
                    "type".to_string(),
                    serde_json::Value::String("file".to_string()),
                ),
                (
                    "extension".to_string(),
                    serde_json::Value::String("pdf".to_string()),
                ),
            ],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(
            &analysis,
            "/app-api/sign-web/file-info",
            &graphql_analysis,
        );

        // Verify template is valid
        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated template should be valid. Template:\n{template}"
        );

        // CRITICAL: Check that download_url uses fake_pdf_data_uri() NOT fake_download_url()
        assert!(
            template.contains("fake_pdf_data_uri"),
            "Box file object with extension: pdf should use fake_pdf_data_uri(). Template:\n{template}"
        );

        assert!(
            !template.contains("fake_download_url()"),
            "Should NOT use generic fake_download_url() for PDFs. Template:\n{template}"
        );

        // Verify constant fields are present
        assert!(template.contains("\"type\": \"file\""));
        assert!(template.contains("\"extension\": \"pdf\""));
    }

    #[test]
    fn test_box_file_object_with_png_download_url() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let sign_object = FieldType::Object(Box::new(crate::type_detector::ObjectAnalysis {
            varying_fields: vec![(
                "download_url".to_string(),
                FieldType::DownloadUrl {
                    sample_url: Some("https://dl.boxcloud.com/d/1/abc123...".to_string()),
                },
            )],
            constant_fields: vec![(
                "download_status".to_string(),
                serde_json::Value::String("success".to_string()),
            )],
        }));

        let analysis = ResponseStructure {
            varying_fields: vec![
                ("id".to_string(), FieldType::NumericStringId),
                ("name".to_string(), FieldType::FileName),
                ("sign".to_string(), sign_object),
            ],
            constant_fields: vec![
                (
                    "type".to_string(),
                    serde_json::Value::String("file".to_string()),
                ),
                (
                    "extension".to_string(),
                    serde_json::Value::String("png".to_string()),
                ),
            ],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/app-api/files", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        assert!(
            template.contains("fake_png_data_uri"),
            "PNG files should use fake_png_data_uri(). Template:\n{template}"
        );

        assert!(!template.contains("fake_download_url()"));
        assert!(template.contains("\"extension\": \"png\""));
    }

    #[test]
    fn test_box_file_object_with_jpeg_download_url() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let sign_object = FieldType::Object(Box::new(crate::type_detector::ObjectAnalysis {
            varying_fields: vec![(
                "download_url".to_string(),
                FieldType::DownloadUrl {
                    sample_url: Some("https://dl.boxcloud.com/d/1/xyz789...".to_string()),
                },
            )],
            constant_fields: vec![(
                "download_status".to_string(),
                serde_json::Value::String("success".to_string()),
            )],
        }));

        let analysis = ResponseStructure {
            varying_fields: vec![
                ("id".to_string(), FieldType::NumericStringId),
                ("name".to_string(), FieldType::FileName),
                ("sign".to_string(), sign_object),
            ],
            constant_fields: vec![
                (
                    "type".to_string(),
                    serde_json::Value::String("file".to_string()),
                ),
                (
                    "extension".to_string(),
                    serde_json::Value::String("jpg".to_string()),
                ),
            ],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/app-api/files", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        assert!(
            template.contains("fake_jpeg_data_uri"),
            "JPEG files should use fake_jpeg_data_uri(). Template:\n{template}"
        );

        assert!(!template.contains("fake_download_url()"));
        assert!(template.contains("\"extension\": \"jpg\""));
    }

    #[test]
    fn test_box_file_with_authenticated_download_url() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let sign_object = FieldType::Object(Box::new(crate::type_detector::ObjectAnalysis {
            varying_fields: vec![(
                "download_url".to_string(),
                FieldType::DownloadUrl {
                    sample_url: Some("https://dl.boxcloud.com/d/1/long_url...".to_string()),
                },
            )],
            constant_fields: vec![(
                "download_status".to_string(),
                serde_json::Value::String("success".to_string()),
            )],
        }));

        let analysis = ResponseStructure {
            varying_fields: vec![
                ("id".to_string(), FieldType::NumericStringId),
                ("sign".to_string(), sign_object),
                // authenticated_download_url is a shorter URL, detected as generic Url
                ("authenticated_download_url".to_string(), FieldType::Url),
            ],
            constant_fields: vec![
                (
                    "type".to_string(),
                    serde_json::Value::String("file".to_string()),
                ),
                (
                    "extension".to_string(),
                    serde_json::Value::String("pdf".to_string()),
                ),
            ],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(
            &analysis,
            "/app-api/sign-web/file-info",
            &graphql_analysis,
        );

        assert!(crate::template::validate_template(&template).is_ok());

        // Both download URLs should use the same PDF data URI generator
        assert!(
            template.contains("fake_pdf_data_uri"),
            "Both download_url and authenticated_download_url should use fake_pdf_data_uri(). Template:\n{template}"
        );

        // Count occurrences - should appear twice (once for each download URL field)
        let count = template.matches("fake_pdf_data_uri").count();
        assert_eq!(
            count, 2,
            "Should have fake_pdf_data_uri() for BOTH download_url and authenticated_download_url. Found {count} occurrences. Template:\n{template}"
        );

        // Should NOT use generic fake_url() or fake_download_url()
        assert!(
            !template.contains("fake_url()"),
            "Should not use fake_url() for download URLs"
        );
        assert!(
            !template.contains("fake_download_url()"),
            "Should not use fake_download_url() for PDFs"
        );
    }

    #[test]
    fn test_all_box_file_extensions_detected() {
        // Test that various extensions work correctly
        for (ext, expected_generator) in &[
            ("pdf", "fake_pdf_data_uri"),
            ("png", "fake_png_data_uri"),
            ("jpg", "fake_jpeg_data_uri"),
            ("jpeg", "fake_jpeg_data_uri"),
        ] {
            let generator = TemplateGenerator::new("api.{path}.total".to_string());

            let sign_object = FieldType::Object(Box::new(crate::type_detector::ObjectAnalysis {
                varying_fields: vec![(
                    "download_url".to_string(),
                    FieldType::DownloadUrl {
                        sample_url: Some("https://dl.boxcloud.com/d/1/test...".to_string()),
                    },
                )],
                constant_fields: vec![],
            }));

            let analysis = ResponseStructure {
                varying_fields: vec![("sign".to_string(), sign_object)],
                constant_fields: vec![
                    (
                        "type".to_string(),
                        serde_json::Value::String("file".to_string()),
                    ),
                    (
                        "extension".to_string(),
                        serde_json::Value::String((*ext).to_string()),
                    ),
                ],
                has_matching_path_ids: false,
                is_json: true,
                top_level_type: "object".to_string(),
                pagination: None,
            };

            let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
            let template =
                generator.generate_tera_template(&analysis, "/api/files", &graphql_analysis);

            assert!(
                template.contains(expected_generator),
                "Extension '{ext}' should use {expected_generator}. Template:\n{template}"
            );
        }
    }

    #[test]
    fn test_generate_tera_template_simple_fields() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let analysis = ResponseStructure {
            varying_fields: vec![
                (
                    "id".to_string(),
                    FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                ),
                ("name".to_string(), FieldType::Name),
                ("email".to_string(), FieldType::Email),
            ],
            constant_fields: vec![(
                "type".to_string(),
                serde_json::Value::String("user".to_string()),
            )],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/users", &graphql_analysis);

        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated template should be valid. Template:\n{template}"
        );

        assert!(template.contains("get_random"));
        assert!(template.contains("fake_name"));
        assert!(template.contains("fake_email"));
        assert!(template.contains("\"type\": \"user\""));
    }

    #[test]
    fn test_categorical_field_generates_valid_template() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let categorical_field = FieldType::Categorical {
            values: vec![
                "document".to_string(),
                "signing_log".to_string(),
                "attachment".to_string(),
            ],
        };

        let analysis = ResponseStructure {
            varying_fields: vec![("type".to_string(), categorical_field)],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/files", &graphql_analysis);

        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Categorical field should generate valid template. Template:\n{template}"
        );

        assert!(template.contains("random_choice"));
        assert!(template.contains("\"document\"") || template.contains("\"signing_log\""));
    }

    #[test]
    fn test_page_based_pagination_with_static_params() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let pagination = PaginationInfo {
            total_field: Some("count".to_string()),
            offset_field: None,
            limit_field: Some("limit".to_string()),
            next_field: Some("next".to_string()),
            prev_field: Some("previous".to_string()),
            has_more_field: None,
            sample_total: Some(80),
            pagination_type: PaginationType::Page,
            static_query_params: "status=active&sort=desc".to_string(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![(
                "results".to_string(),
                FieldType::Array(Box::new(ArrayPattern {
                    is_homogeneous: true,
                    element_type: FieldType::Object(Box::new(
                        crate::type_detector::ObjectAnalysis {
                            varying_fields: vec![(
                                "id".to_string(),
                                FieldType::RandomNumber {
                                    min: None,
                                    max: None,
                                },
                            )],
                            constant_fields: vec![],
                        },
                    )),
                    sample_size_range: (5, 10),
                })),
            )],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/items", &graphql_analysis);

        // Verify template is valid
        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated template should be valid. Template:\n{template}"
        );

        // Verify pagination variables are defined
        assert!(template.contains("set limit = query.limit"));
        assert!(template.contains("set page = query.page"));
        assert!(template.contains("set total = store_get_or_set"));
        assert!(template.contains("set has_more = page < total_pages"));

        // Verify count field uses total variable
        assert!(template.contains("\"count\": {{ total }}"));

        // Verify next URL includes static params and dynamic page/limit
        assert!(template.contains("\"next\":"));
        assert!(template.contains("status=active&sort=desc"));
        assert!(template.contains("page={{ page + 1 }}"));
        assert!(template.contains("limit={{ limit }}"));

        // Verify previous URL includes static params
        assert!(template.contains("\"previous\":"));
        assert!(template.contains("if page > 1"));

        // Verify results array uses limit variable
        assert!(template.contains("range(end=limit)"));

        // Verify NO duplicate next field
        let next_count = template.matches("\"next\":").count();
        assert_eq!(
            next_count, 1,
            "Should have exactly one 'next' field, found {next_count}"
        );
    }

    #[test]
    fn test_page_based_pagination_without_static_params() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let pagination = PaginationInfo {
            total_field: Some("total".to_string()),
            offset_field: None,
            limit_field: None,
            next_field: Some("next".to_string()),
            prev_field: Some("prev".to_string()),
            has_more_field: None,
            sample_total: Some(100),
            pagination_type: PaginationType::Page,
            static_query_params: String::new(), // No static params
        };

        let analysis = ResponseStructure {
            varying_fields: vec![(
                "items".to_string(),
                FieldType::Array(Box::new(ArrayPattern {
                    is_homogeneous: true,
                    element_type: FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                    sample_size_range: (10, 20),
                })),
            )],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/data", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        // Verify next URL has only page and limit params (no static params)
        assert!(template.contains("\"next\":"));
        assert!(template.contains("?page={{ page + 1 }}&limit={{ limit }}"));

        // Should NOT contain double query params separator
        assert!(!template.contains("??"));
        assert!(!template.contains("?&"));
    }

    #[test]
    fn test_offset_based_pagination_with_static_params() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let pagination = PaginationInfo {
            total_field: Some("total_count".to_string()),
            offset_field: Some("offset".to_string()),
            limit_field: Some("limit".to_string()),
            next_field: Some("next_url".to_string()),
            prev_field: Some("prev_url".to_string()),
            has_more_field: None,
            sample_total: Some(500),
            pagination_type: PaginationType::Offset,
            static_query_params: "filter=active&include=metadata".to_string(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/api/records", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        // Verify offset pagination variables
        assert!(template.contains("set offset = query.offset"));
        assert!(template.contains("set limit = query.limit"));

        // Verify next URL includes static params and offset calculation
        assert!(template.contains("filter=active&include=metadata"));
        assert!(template.contains("offset={{ offset + limit }}"));

        // Verify previous URL uses max filter to prevent negative offset
        assert!(template.contains("[0, offset - limit] | max"));
    }

    #[test]
    fn test_cursor_based_pagination() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let pagination = PaginationInfo {
            total_field: Some("count".to_string()),
            offset_field: None,
            limit_field: Some("page_size".to_string()),
            next_field: Some("next_cursor".to_string()),
            prev_field: Some("prev_cursor".to_string()),
            has_more_field: None,
            sample_total: Some(200),
            pagination_type: PaginationType::Cursor,
            static_query_params: String::new(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/api/stream", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        // Verify cursor pagination uses page_num variable
        assert!(template.contains("set page_num = store_incr"));

        // Verify cursor format (not URLs)
        assert!(template.contains("cursor_page_"));
        assert!(template.contains("uuid() | truncate(length=8, end=\"\")"));

        // Cursor pagination should generate tokens, not full URLs
        assert!(template.contains("\"next_cursor\":"));
        assert!(!template.contains("fake_api_url()") || !template.contains("next_cursor"));
    }

    #[test]
    fn test_results_array_uses_limit_variable() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let pagination = PaginationInfo {
            total_field: Some("count".to_string()),
            offset_field: None,
            limit_field: Some("limit".to_string()),
            next_field: Some("next".to_string()),
            prev_field: None,
            has_more_field: None,
            sample_total: Some(50),
            pagination_type: PaginationType::Page,
            static_query_params: String::new(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![
                (
                    "results".to_string(),
                    FieldType::Array(Box::new(ArrayPattern {
                        is_homogeneous: true,
                        element_type: FieldType::Name,
                        sample_size_range: (10, 20),
                    })),
                ),
                (
                    "metadata".to_string(),
                    FieldType::Array(Box::new(ArrayPattern {
                        is_homogeneous: true,
                        element_type: FieldType::RandomNumber {
                            min: None,
                            max: None,
                        },
                        sample_size_range: (5, 10),
                    })),
                ),
            ],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/api/search", &graphql_analysis);

        assert!(crate::template::validate_template(&template).is_ok());

        // The "results" field should use limit variable
        assert!(
            template.contains("\"results\":") && template.contains("range(end=limit)"),
            "Results array should use limit variable. Template:\n{template}"
        );

        // The "metadata" field should use get_random (not a recognized pagination results field)
        assert!(
            template.contains("\"metadata\":") && template.contains("get_random"),
            "Non-results array should use get_random. Template:\n{template}"
        );
    }

    // GraphQL Template Generation Tests

    #[test]
    fn test_graphql_template_with_variable_extraction() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let analysis = ResponseStructure {
            varying_fields: vec![
                (
                    "data.user.id".to_string(),
                    FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                ),
                ("data.user.name".to_string(), FieldType::Name),
                ("data.user.email".to_string(), FieldType::Email),
            ],
            constant_fields: vec![(
                "data.user.type".to_string(),
                serde_json::Value::String("user".to_string()),
            )],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        // GraphQL analysis shows "id" is a varying variable
        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo {
            varying_variables: vec!["id".to_string()],
            constant_variables: vec![],
            has_variables: true,
            has_varying_variables: true,
        };

        let template = generator.generate_tera_template(&analysis, "/graphql", &graphql_analysis);

        // The template should use body_json.variables.id for the id field
        assert!(
            template.contains("{{ body_json.variables.id }}"),
            "Template should use GraphQL variable extraction for id. Template:\n{template}"
        );

        // Other fields should still use fake data
        assert!(
            template.contains("fake_name()") || template.contains("name"),
            "Template should have name field. Template:\n{template}"
        );

        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated GraphQL template should be valid. Template:\n{template}"
        );
    }

    #[test]
    fn test_graphql_template_with_multiple_variables() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let analysis = ResponseStructure {
            varying_fields: vec![
                (
                    "data.post.id".to_string(),
                    FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                ),
                (
                    "data.post.userId".to_string(),
                    FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                ),
                ("data.post.title".to_string(), FieldType::Name),
            ],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        // Multiple varying variables
        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo {
            varying_variables: vec!["id".to_string(), "userId".to_string()],
            constant_variables: vec![],
            has_variables: true,
            has_varying_variables: true,
        };

        let template = generator.generate_tera_template(&analysis, "/graphql", &graphql_analysis);

        // Both variables should be extracted from request
        assert!(
            template.contains("{{ body_json.variables.id }}"),
            "Template should extract id variable. Template:\n{template}"
        );
        assert!(
            template.contains("{{ body_json.variables.userId }}"),
            "Template should extract userId variable. Template:\n{template}"
        );

        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated GraphQL template should be valid. Template:\n{template}"
        );
    }

    #[test]
    fn test_graphql_template_with_no_matching_variables() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        let analysis = ResponseStructure {
            varying_fields: vec![
                (
                    "data.result.score".to_string(),
                    FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                ),
                ("data.result.status".to_string(), FieldType::Name),
            ],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: None,
        };

        // Variables that don't match response fields
        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo {
            varying_variables: vec!["userId".to_string()],
            constant_variables: vec![],
            has_variables: true,
            has_varying_variables: true,
        };

        let template = generator.generate_tera_template(&analysis, "/graphql", &graphql_analysis);

        // Should NOT use variable extraction since field names don't match
        assert!(
            !template.contains("{{ body_json.variables."),
            "Template should not use variable extraction when fields don't match. Template:\n{template}"
        );

        // Should use regular fake data
        assert!(
            template.contains("random(") || template.contains("score"),
            "Template should use fake data for score. Template:\n{template}"
        );

        assert!(
            crate::template::validate_template(&template).is_ok(),
            "Generated template should be valid. Template:\n{template}"
        );
    }

    #[test]
    fn test_no_duplicate_next_field() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        // This test specifically addresses the bug where "next" was matched as both
        // next_field and has_more_field, causing duplicate entries
        let pagination = PaginationInfo {
            total_field: Some("count".to_string()),
            offset_field: None,
            limit_field: Some("limit".to_string()),
            next_field: Some("next".to_string()),
            prev_field: Some("previous".to_string()),
            has_more_field: None, // Should NOT match "next" as has_more_field
            sample_total: Some(80),
            pagination_type: PaginationType::Page,
            static_query_params: "foo=bar".to_string(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/docs", &graphql_analysis);

        // Count occurrences of "next" field definition
        let next_count = template.matches("\"next\":").count();
        assert_eq!(
            next_count, 1,
            "Should have exactly one 'next' field definition, found {next_count}. Template:\n{template}"
        );

        // Verify the next field is the URL version, not a boolean
        assert!(
            template.contains("\"next\": {% if has_more %}\"{{ fake_api_url()"),
            "Next field should be URL-based"
        );
        assert!(
            !template.contains("\"next\": {{ has_more }}"),
            "Next field should NOT be boolean has_more"
        );
    }

    #[test]
    fn test_cursor_pagination_without_total_field() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        // Test cursor pagination when API doesn't provide a total field
        // This was causing "undefined variable 'total'" errors before the fix
        let pagination = PaginationInfo {
            total_field: None, // No total field in response
            offset_field: None,
            limit_field: Some("limit".to_string()),
            next_field: Some("next_marker".to_string()),
            prev_field: None,
            has_more_field: None,
            sample_total: None, // No sample total available
            pagination_type: PaginationType::Cursor,
            static_query_params: String::new(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![(
                "entries".to_string(),
                FieldType::Array(Box::new(ArrayPattern {
                    is_homogeneous: true,
                    element_type: FieldType::RandomNumber {
                        min: None,
                        max: None,
                    },
                    sample_size_range: (20, 100),
                })),
            )],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/api/templates", &graphql_analysis);

        // Template should validate without errors
        assert!(crate::template::validate_template(&template).is_ok());

        // Should use page_num counter instead of total
        assert!(template.contains("set page_num = store_incr"));

        // Should use max page limit instead of total-based calculation
        assert!(template.contains("set has_more = page_num < 10"));

        // Should NOT reference undefined 'total' variable
        assert!(!template.contains("< total"));
        assert!(!template.contains("total / limit"));

        // Should still generate next_marker correctly
        assert!(template.contains("\"next_marker\":"));
        assert!(template.contains("cursor_page_"));
    }

    #[test]
    fn test_page_pagination_without_total_field() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        // Test page-based pagination without total field
        let pagination = PaginationInfo {
            total_field: None, // No total field
            offset_field: None,
            limit_field: Some("per_page".to_string()),
            next_field: Some("next_page_url".to_string()),
            prev_field: Some("prev_page_url".to_string()),
            has_more_field: None,
            sample_total: None,
            pagination_type: PaginationType::Page,
            static_query_params: String::new(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template = generator.generate_tera_template(&analysis, "/api/items", &graphql_analysis);

        // Template should validate
        assert!(crate::template::validate_template(&template).is_ok());

        // Should use page limit instead of total_pages
        assert!(template.contains("set has_more = page < 10"));

        // Should NOT reference undefined variables
        assert!(!template.contains("total_pages"));
        assert!(!template.contains("total / limit"));
        assert!(!template.contains("< total"));

        // Should still generate page URLs
        assert!(template.contains("\"next_page_url\":"));
        assert!(template.contains("page={{ page + 1 }}"));
    }

    #[test]
    fn test_offset_pagination_without_total_field() {
        let generator = TemplateGenerator::new("api.{path}.total".to_string());

        // Test offset-based pagination without total field and with has_more field
        let pagination = PaginationInfo {
            total_field: None, // No total field
            offset_field: Some("offset".to_string()),
            limit_field: Some("limit".to_string()),
            next_field: None,
            prev_field: None,
            has_more_field: Some("has_more".to_string()), // has_more field present
            sample_total: None,
            pagination_type: PaginationType::Offset,
            static_query_params: String::new(),
        };

        let analysis = ResponseStructure {
            varying_fields: vec![],
            constant_fields: vec![],
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "object".to_string(),
            pagination: Some(pagination),
        };

        let graphql_analysis = crate::codegen::types::GraphQLVariableInfo::empty();
        let template =
            generator.generate_tera_template(&analysis, "/api/records", &graphql_analysis);

        // Template should validate
        assert!(crate::template::validate_template(&template).is_ok());

        // Should use max offset limit instead of total
        assert!(template.contains("set has_more = offset < 10000"));

        // Should NOT reference undefined 'total' variable
        assert!(!template.contains("(offset + limit) < total"));
        assert!(!template.contains("total / limit"));

        // Should still include offset and limit
        assert!(template.contains("set offset = query.offset"));
        assert!(template.contains("set limit = query.limit"));
    }
}
