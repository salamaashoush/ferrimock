//! Template access to the entity world.
//!
//! The typed counterpart of the `store_*` functions: `store_get` reads a value
//! somebody put there, `entity_get` reads an instance of a type the API
//! declares. Both reach process-global state for the same reason — Tera's
//! function registry is stateless, so there is nowhere to thread a handle
//! through.
//!
//! These read and write the *same* store a schema-derived route serves, which
//! is the point: a declarative template can answer an endpoint the schema does
//! not cover while still seeing the entities the schema does.

use serde_json::Value as JsonValue;
use tera::{Kwargs, State, TeraResult, Value as TeraValue};

use crate::core::{EntityPage, EntityQuery, World, global_world};

fn world() -> std::sync::Arc<World> {
    global_world()
}

/// Register every `entity_*` function with a Tera instance.
pub fn register_all_functions(tera: &mut tera::Tera) {
    // entity_types() -> ["User", "Folder", ...]
    tera.register_function(
        "entity_types",
        |_: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let names: Vec<String> = world()
                .entities()
                .into_iter()
                .map(|name| name.to_string())
                .collect();
            Ok(super::convert::to_tera(serde_json::json!(names)))
        },
    );

    // entity_count(type)
    tera.register_function(
        "entity_count",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<usize> {
            let entity = kwargs.must_get::<&str>("type")?;
            Ok(world().count(entity))
        },
    );

    // entity_get(type, key) -> object, or none when it never existed or was removed
    tera.register_function(
        "entity_get",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let entity = kwargs.must_get::<&str>("type")?;
            let key = kwargs.must_get::<&str>("key")?;
            Ok(world()
                .get(entity, key)
                .map_or_else(TeraValue::none, super::convert::to_tera))
        },
    );

    // entity_list(type, filter={}, sort="-createdAt", skip=0, limit=25)
    tera.register_function(
        "entity_list",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let entity = kwargs.must_get::<&str>("type")?;
            let query = query_from(&kwargs)?;
            let page = world()
                .list(entity, &query)
                .map_err(|e| tera::Error::message(format!("entity_list: {e}")))?;
            Ok(super::convert::to_tera(page_json(&page)))
        },
    );

    // entity_related(type, key, field, filter={}, sort=..., skip=, limit=)
    tera.register_function(
        "entity_related",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let entity = kwargs.must_get::<&str>("type")?;
            let key = kwargs.must_get::<&str>("key")?;
            let field = kwargs.must_get::<&str>("field")?;
            let query = query_from(&kwargs)?;
            let page = world()
                .related(entity, key, field, &query)
                .map_err(|e| tera::Error::message(format!("entity_related: {e}")))?;
            Ok(super::convert::to_tera(page_json(&page)))
        },
    );

    // entity_create(type, values={}) -> the created instance
    tera.register_function(
        "entity_create",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let entity = kwargs.must_get::<&str>("type")?;
            let values = values_from(&kwargs, "values")?;
            let created = world()
                .create(entity, values)
                .map_err(|e| tera::Error::message(format!("entity_create: {e}")))?;
            Ok(super::convert::to_tera(created))
        },
    );

    // entity_update(type, key, values={}) -> the updated instance
    tera.register_function(
        "entity_update",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let entity = kwargs.must_get::<&str>("type")?;
            let key = kwargs.must_get::<&str>("key")?;
            let values = values_from(&kwargs, "values")?;
            let updated = world()
                .update(entity, key, values)
                .map_err(|e| tera::Error::message(format!("entity_update: {e}")))?;
            Ok(super::convert::to_tera(updated))
        },
    );

    // entity_delete(type, key)
    tera.register_function(
        "entity_delete",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<String> {
            let entity = kwargs.must_get::<&str>("type")?;
            let key = kwargs.must_get::<&str>("key")?;
            world()
                .delete(entity, key)
                .map_err(|e| tera::Error::message(format!("entity_delete: {e}")))?;
            // Empty, so `{{ entity_delete(...) }}` leaves nothing in the body.
            Ok(String::new())
        },
    );
}

fn query_from(kwargs: &Kwargs) -> TeraResult<EntityQuery> {
    let filter = match kwargs.get::<&TeraValue>("filter")? {
        Some(value) => match super::convert::to_json(value) {
            JsonValue::Object(map) => map,
            other => {
                return Err(tera::Error::message(format!(
                    "`filter` has to be an object of field to value, got {other}"
                )));
            }
        },
        None => serde_json::Map::new(),
    };

    let sort = kwargs
        .get::<&str>("sort")?
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Ok(EntityQuery {
        filter,
        sort,
        skip: kwargs.get::<usize>("skip")?.unwrap_or(0),
        limit: kwargs.get::<usize>("limit")?,
    })
}

fn values_from(kwargs: &Kwargs, name: &str) -> TeraResult<JsonValue> {
    match kwargs.get::<&TeraValue>(name)? {
        None => Ok(JsonValue::Object(serde_json::Map::new())),
        Some(value) => match super::convert::to_json(value) {
            object @ JsonValue::Object(_) => Ok(object),
            other => Err(tera::Error::message(format!(
                "`{name}` has to be an object, got {other}"
            ))),
        },
    }
}

/// A page in the shape a template reads: `.records`, `.total`, `.has_next`.
fn page_json(page: &EntityPage) -> JsonValue {
    serde_json::json!({
        "records": page.records.clone(),
        "total": page.total,
        "hasNext": page.has_next,
        "hasPrevious": page.has_previous,
    })
}

// The fixture is a schema, and only the `spec` feature can load one.
#[cfg(all(test, feature = "spec"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::core::global_world;
    use crate::types::RequestContext;

    /// Every entity these tests use, declared in one schema.
    ///
    /// Loaded exactly once: adding a schema rebuilds the store, and a rebuild
    /// racing another test's write would drop it. Each test still gets its own
    /// entity so the tests stay independent of each other's writes.
    const SCHEMA: &str = "
        type TemplateCounted { id: ID!, name: String! }
        type TemplateListed { id: ID!, name: String! }
        type TemplateWritten { id: ID!, name: String! }
        type TemplateFiltered { id: ID!, name: String! }
        type Query {
          counted: [TemplateCounted!]!
          listed: [TemplateListed!]!
          written: [TemplateWritten!]!
          filtered: [TemplateFiltered!]!
        }
    ";

    fn seed() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            crate::spec::source::load_schema(
                SCHEMA,
                std::path::Path::new("template_entities.graphql"),
                &global_world(),
                false,
            )
            .unwrap();
        });
    }

    fn render(template: &str) -> String {
        crate::template::render_template(template, &RequestContext::new()).unwrap()
    }

    #[test]
    fn a_template_counts_entities() {
        seed();
        let rendered = render(r#"{{ entity_count(type="TemplateCounted") }}"#);
        assert_eq!(
            rendered,
            crate::core::world::store::DEFAULT_SEED_COUNT.to_string()
        );
    }

    #[test]
    fn a_template_reads_a_listed_instance_back_by_key() {
        seed();
        let key = render(
            r#"{% set page = entity_list(type="TemplateListed", limit=1) %}{{ page.records[0].id }}"#,
        );
        let name = render(&format!(
            r#"{{% set found = entity_get(type="TemplateListed", key="{key}") %}}{{{{ found.name }}}}"#
        ));
        assert!(
            !name.is_empty(),
            "an entity read by key must have its fields"
        );
    }

    #[test]
    fn a_template_write_is_visible_to_the_next_read() {
        seed();
        let created = render(
            r#"{% set made = entity_create(type="TemplateWritten", values={"name": "from-a-template"}) %}{{ made.id }}"#,
        );
        let name = render(&format!(
            r#"{{% set found = entity_get(type="TemplateWritten", key="{created}") %}}{{{{ found.name }}}}"#
        ));
        assert_eq!(name, "from-a-template");
    }

    #[test]
    fn a_filter_narrows_a_list() {
        seed();
        global_world()
            .create("TemplateFiltered", serde_json::json!({ "name": "needle" }))
            .unwrap();

        let total = render(
            r#"{% set page = entity_list(type="TemplateFiltered", filter={"name": "needle"}) %}{{ page.total }}"#,
        );
        assert_eq!(total, "1");
    }

    #[test]
    fn an_unknown_entity_is_a_template_error_not_an_empty_page() {
        seed();
        let result = crate::template::render_template(
            r#"{% set page = entity_list(type="NoSuchEntity") %}{{ page.total }}"#,
            &RequestContext::new(),
        );
        assert!(result.is_err(), "a typo must not read as an empty result");
    }
}
