//! Expanding a `serve:` mock into the routes that serve the world.
//!
//! A `serve:` entry is a *family* of mocks written as one. GraphQL makes a
//! family of one — the endpoint, matching any operation, because a schema
//! cannot know the operation names a client will choose. A protocol that
//! designs many endpoints expands to one mock per operation instead, so
//! coverage names the endpoints and a hand-written mock overrides exactly one
//! of them.

use std::sync::Arc;

use super::parser::ServeConfig;
use crate::core::World;
use crate::types::MockDefinition;

/// Protocols `serve:` understands, for the error when it does not.
const KNOWN_PROTOCOLS: [&str; 1] = ["graphql"];

/// Priority a schema-derived route takes.
///
/// Below the default 100, so a mock written by hand outranks the backend
/// without anyone having to think about numbers.
pub const SERVED_PRIORITY: u32 = 50;

/// Expand one `serve:` mock.
///
/// The definition arrives fully lowered — URL, priority, scope, headers, delay
/// — because those are the mock's business, not the schema's.
pub fn expand(
    mock: MockDefinition,
    serve: &ServeConfig,
    world: &Arc<World>,
) -> crate::Result<Vec<MockDefinition>> {
    match serve.protocol() {
        "graphql" => expand_graphql(mock, serve, world),
        "rest" | "openapi" => Err(crate::mp_err!(
            "mock `{}`: `serve: rest` needs an OpenAPI front end, which is not built yet. \
             Only {} can be served today.",
            mock.id,
            KNOWN_PROTOCOLS.join(", ")
        )),
        other => Err(crate::mp_err!(
            "mock `{}`: `{other}` is not a protocol `serve:` understands (known: {})",
            mock.id,
            KNOWN_PROTOCOLS.join(", ")
        )),
    }
}

#[cfg(feature = "spec")]
fn expand_graphql(
    mut mock: MockDefinition,
    serve: &ServeConfig,
    world: &Arc<World>,
) -> crate::Result<Vec<MockDefinition>> {
    use crate::core::world::Binding;
    use crate::spec::bind::graphql::GraphQLBackend;

    let schema = world.resolve_schema("graphql", serve.schema(), mock.id.as_str())?;
    let Binding::GraphQL(parsed) = &schema.binding else {
        return Err(crate::mp_err!(
            "mock `{}`: {} is not a GraphQL schema",
            mock.id,
            schema.path.display()
        ));
    };

    // One backend per mount. Two mounts of the same schema build two
    // executable schemas over one world, so they serve identical data — which
    // is what sharing the world means, and cheaper to keep than to cache.
    let backend = Arc::new(
        GraphQLBackend::build(parsed, Arc::clone(world)).map_err(|e| {
            crate::mp_err!(
                "mock `{}`: could not build an executable schema from {}: {e}",
                mock.id,
                schema.path.display()
            )
        })?,
    );

    // `source_file` stays whatever the caller set: the mock is declared in a
    // collection, and that is the file `reload_file` knows how to re-run.
    // Pointing it at the schema would make a reload try to rebuild routes from
    // a file that declares no routes, and they would silently disappear.
    crate::spec::emit::bind_graphql(&mut mock, backend);
    Ok(vec![mock])
}

#[cfg(not(feature = "spec"))]
fn expand_graphql(
    mock: MockDefinition,
    _serve: &ServeConfig,
    _world: &Arc<World>,
) -> crate::Result<Vec<MockDefinition>> {
    Err(crate::mp_err!(
        "mock `{}`: serving a schema needs the `spec` feature",
        mock.id
    ))
}

/// The priority a `serve:` mock takes when its config did not name one.
///
/// A schema-derived route sits below the default so a mock written by hand
/// outranks it without anyone doing arithmetic. A config that spells out the
/// default priority is read as not having chosen — the alternative is making
/// `priority` optional across the whole format for one case that does not
/// arise.
#[must_use]
pub fn priority_for(configured: u32) -> u32 {
    if configured == super::parser::DEFAULT_PRIORITY {
        SERVED_PRIORITY
    } else {
        configured
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_default_priority_drops_below_hand_written_mocks() {
        assert_eq!(
            priority_for(super::super::parser::DEFAULT_PRIORITY),
            SERVED_PRIORITY
        );
    }

    #[test]
    fn an_explicit_priority_is_kept() {
        assert_eq!(priority_for(250), 250);
    }

    #[test]
    fn an_unknown_protocol_names_the_known_ones() {
        let world = Arc::new(World::new());
        let mock = crate::config::MockConfig {
            id: "x".into(),
            match_config: Some(crate::config::MatchConfig {
                url: Some("/graphql".to_string()),
                ..crate::config::MatchConfig::default()
            }),
            ..crate::config::MockConfig::default()
        };
        let error = expand(
            futures::executor::block_on(mock.into_mock_definition_with_dir(None)).unwrap(),
            &ServeConfig::Protocol("soap".to_string()),
            &world,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("soap"), "unexpected: {error}");
        assert!(error.contains("graphql"), "unexpected: {error}");
    }
}
