//! Custom Tera function registration

use tera::Tera;

/// Register all custom Tera functions for mock templates
pub fn register_custom_functions(tera: &mut Tera) {
    super::filters::register_all_filters(tera);
    super::fake_data::register_all_functions(tera);
    super::store::register_all_functions(tera);
    super::graphql_helpers::register_all_functions(tera);

    // Plugin functions registered by embedders via register_template_function()
    super::plugin::apply_plugins(tera);
}
