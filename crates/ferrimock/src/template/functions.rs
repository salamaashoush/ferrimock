//! Custom Tera function registration

use rand::{RngExt, SeedableRng};
use std::hash::{DefaultHasher, Hash, Hasher};
use tera::{Kwargs, State, Tera, TeraResult};

/// Register all custom Tera functions for mock templates
pub fn register_custom_functions(tera: &mut Tera) {
    register_contrib(tera);
    super::filters::register_all_filters(tera);
    super::fake_data::register_all_functions(tera);
    super::store::register_all_functions(tera);
    super::entities::register_all_functions(tera);
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
    tera.register_function("get_random", get_random);
}

/// `tera_contrib::rand::get_random`, except the unseeded branch draws from the
/// ferrimock stream so a global seed makes it reproducible. The explicit-`seed`
/// branch keeps contrib's `StdRng` so existing templates render unchanged.
// Tera fixes the signature; `kwargs` cannot be taken by reference.
#[allow(clippy::needless_pass_by_value)]
fn get_random(kwargs: Kwargs, _state: &State<'_>) -> TeraResult<i64> {
    let start = kwargs.must_get::<i64>("start")?;
    let end = kwargs.must_get::<i64>("end")?;

    if start >= end {
        return Err(tera::Error::message(format!(
            "get_random: `start` ({start}) must be less than `end` ({end})."
        )));
    }

    match kwargs.get::<String>("seed")? {
        Some(seed) => {
            let mut hasher = DefaultHasher::new();
            seed.hash(&mut hasher);
            let mut rng = rand::rngs::StdRng::seed_from_u64(hasher.finish());
            Ok(rng.random_range(start..end))
        }
        None => Ok(crate::fake_data::rng::rng().random_range(start..end)),
    }
}
