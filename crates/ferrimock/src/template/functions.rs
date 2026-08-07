//! Custom Tera function registration

use tera::Tera;

/// Register all custom Tera functions for mock templates
pub fn register_custom_functions(tera: &mut Tera) {
    register_contrib(tera);
    super::filters::register_all_filters(tera);
    super::fake_data::register_all_functions(tera);
    super::store::register_all_functions(tera);
    super::graphql_helpers::register_all_functions(tera);

    // Plugin functions registered by embedders via register_template_function()
    super::plugin::apply_plugins(tera);
}

/// Tera 2 moved its dependency-heavy built-ins into `tera-contrib`. These four
/// are part of the documented mock template surface, so re-register them.
fn register_contrib(tera: &mut Tera) {
    tera.register_filter("json_encode", tera_contrib::json::json_encode);
    tera.register_filter("date", tera_contrib::dates::date);
    tera.register_function("now", tera_contrib::dates::now);
    tera.register_function("get_random", tera_contrib::rand::get_random);
}
