//! Custom Tera function registration

use tera::Tera;

/// Register custom Tera functions for mock templates
pub fn register_custom_functions(tera: &mut Tera) {
    // ============================================================================
    // REGISTER CUSTOM FILTERS
    // ============================================================================
    super::filters::register_all_filters(tera);

    // ============================================================================
    // FAKE DATA GENERATORS (using shared fake_data module)
    // ============================================================================

    // Register all fake data generation functions
    super::fake_data::register_all_functions(tera);

    // ============================================================================
    // PERSISTENCE API (store functions)
    // ============================================================================

    // Register all persistence store functions
    super::store::register_all_functions(tera);

    // ============================================================================
    // GRAPHQL HELPERS
    // ============================================================================

    // Register all GraphQL helper functions
    super::graphql_helpers::register_all_functions(tera);
}
