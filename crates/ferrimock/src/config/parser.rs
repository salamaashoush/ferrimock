//! Mock collection configuration and format parsing

use super::har::HarLoader;
use super::matcher::MatchConfig;
use super::request_transform::RequestTransformConfig;
use super::response::{
    ResponseConfig, ResponsePatchesConfig, parse_duration, parse_patches_config,
};
use super::serve::Behaviour;
use crate::Result;
use crate::types::MockDefinition;
use lean_string::LeanString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Mock collection configuration (top-level structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MockCollectionConfig {
    /// Collection metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default = "default_enabled", skip_serializing_if = "is_true")]
    pub enabled: bool,

    /// Collection-level variables available in all mock templates as {{ vars.key }}
    /// These shadow global vars and are shadowed by mock-level vars
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::HashMap<String, serde_json::Value>>")
    )]
    pub vars: Option<serde_json::Map<String, serde_json::Value>>,

    /// List of mock definitions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mocks: Vec<MockConfig>,

    /// The entity world this collection contributes to.
    ///
    /// Declares *entities*, not routes — the way `vars` declares values rather
    /// than mocks. There is one world per process, so several collections may
    /// add schemas to it but only one may set its seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldConfig>,
}

/// A collection's contribution to the entity world.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WorldConfig {
    /// Schemas to read entities from, relative to the collection file.
    ///
    /// A `.graphql` beside the collection is picked up without being listed;
    /// anything with an ordinary extension has to be named here, because a
    /// `.yaml` is a mock collection and guessing between the two by sniffing
    /// content is the kind of magic that breaks silently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<String>,

    /// Seed for the generated world. Defaults to the process seed (`--seed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,

    /// Instances per entity when the entity does not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,

    /// Multiplies whatever the default count resolves to.
    ///
    /// A schema does not say how big its world should be, and the answer is
    /// different for a unit test and for a screen someone is looking at. This
    /// is how a mount asks for a bigger one without naming every entity in it;
    /// a count stated per entity is what the caller said and is left alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,

    /// Per-entity instance counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::HashMap<String, usize>>")
    )]
    pub counts: Option<std::collections::BTreeMap<String, usize>>,

    /// Where to keep writes so the world outlives the process.
    ///
    /// The file holds the delta — the writes laid over the seed — not the
    /// entities, because the entities are derived and a seed already
    /// reproduces them exactly. Relative to the collection file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence: Option<String>,

    /// Repair malformed schemas rather than refusing them.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lenient: bool,

    /// Whether removing a record also removes what points at it.
    ///
    /// On, a `DELETE` takes the children with it, the way a database with
    /// `ON DELETE CASCADE` would. Off, a delete that would orphan children is
    /// refused — which is what an API enforcing referential integrity does, and
    /// what a test asserting on that behaviour needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade_delete: Option<bool>,

    /// What a field should hold, keyed by `Entity.field` or `*.field`.
    ///
    /// A schema types a field as `String` and stops; which string it is remains
    /// a product decision. `*.field` reaches every entity that has it, and
    /// `Entity.field` beats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, serde_json::Value>>")
    )]
    pub fields: Option<std::collections::BTreeMap<String, FieldOverride>>,

    /// The same, keyed by a *kind* of value rather than a place: a GraphQL
    /// custom scalar (`Money`) or an OpenAPI `format` (`date-time`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, serde_json::Value>>")
    )]
    pub scalars: Option<std::collections::BTreeMap<String, FieldOverride>>,

    /// Which entity a request's credential is an instance of.
    ///
    /// A root field returning one instance with no way to say which — `viewer`,
    /// `me`, `currentUser` — is the one endpoint whose whole purpose is to be
    /// different per caller, and a schema cannot say who the caller is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer: Option<String>,

    /// What a status field means, keyed by `Entity.field`.
    ///
    /// A sequence rather than a mapping, because the order *is* the lifecycle:
    /// a record moves to a later state and never to an earlier one, and a YAML
    /// mapping does not promise to keep the order it was written in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::BTreeMap<String, serde_json::Value>>")
    )]
    pub states: Option<std::collections::BTreeMap<String, Vec<StateConfig>>>,
}

/// What an override says a field holds.
///
/// A bare string is either a generator name (`email`) or, when it looks like
/// one, a Tera template (`"{{ fake_word() | upper }}"`). The template is the
/// escape hatch: it costs a render per value and only the fields that ask for
/// it pay, which is why the named form exists at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldOverride {
    /// `email`, or `"{{ ... }}"`.
    Named(String),
    /// A shape with parameters.
    Shaped(ShapedOverride),
}

/// The parameterised overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapedOverride {
    /// `{ int: { min: 1, max: 10 } }`
    Int(NumberRange),
    /// `{ float: { min: 1.0, max: 99.99 } }`
    Float(NumberRange),
    /// `{ one_of: [pending, shipped] }`
    OneOf(Vec<String>),
    /// `{ constant: "ADD_METADATA" }`
    Constant(serde_json::Value),
    /// `{ pattern: "^[A-Z]{3}$" }`
    Pattern(String),
}

/// Bounds on a generated number.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NumberRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl WorldConfig {
    /// Read the `fields:` and `scalars:` blocks into rules the world applies.
    ///
    /// A name nothing answers to is an error rather than a field that quietly
    /// keeps whatever was inferred — a typo in a mapping file should be found
    /// when the file is read, not from a payload that looks almost right.
    pub fn field_rules(&self) -> crate::Result<crate::core::world::overrides::FieldRules> {
        use crate::core::world::overrides::{FieldRules, RuleKey};

        let mut rules = FieldRules::default();
        for (target, stated) in self.fields.iter().flatten() {
            rules.insert(parse_target(target), resolve(target, stated)?);
        }
        for (declared, stated) in self.scalars.iter().flatten() {
            rules.insert(
                RuleKey::Declared(LeanString::from(declared.as_str())),
                resolve(declared, stated)?,
            );
        }
        for (target, states) in self.states.iter().flatten() {
            rules.insert(parse_target(target), lifecycle_of(target, states)?);
        }
        Ok(rules)
    }
}

/// One state of a lifecycle, and what a record in it does not carry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StateConfig {
    pub name: String,

    /// How much of the population sits here.
    #[serde(default = "one")]
    pub weight: f64,

    /// Fields a record in this state does not have. An order that has not
    /// shipped has no `shipped_at`, and a payload carrying one is a
    /// contradiction rather than an unlikely value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub empty: Vec<String>,
}

const fn one() -> f64 {
    1.0
}

/// `User.email`, `*.email` or a bare `email` — all three name a place.
fn parse_target(target: &str) -> crate::core::world::overrides::RuleKey {
    use crate::core::world::overrides::RuleKey;

    match target.split_once('.') {
        Some(("*", field)) => RuleKey::AnyEntity(LeanString::from(field)),
        Some((entity, field)) => RuleKey::Field {
            entity: LeanString::from(entity),
            field: LeanString::from(field),
        },
        None => RuleKey::AnyEntity(LeanString::from(target)),
    }
}

fn lifecycle_of(
    target: &str,
    states: &[StateConfig],
) -> crate::Result<crate::core::world::overrides::FieldRule> {
    use crate::core::world::model::{Lifecycle, LifecycleState};
    use crate::core::world::overrides::FieldRule;

    if states.len() < 2 {
        return Err(crate::mp_err!(
            "`{target}` is a lifecycle, so it needs at least two states"
        ));
    }
    Ok(FieldRule::Lifecycle(Lifecycle {
        states: states
            .iter()
            .map(|state| LifecycleState {
                name: LeanString::from(state.name.as_str()),
                weight: state.weight,
                empty: state
                    .empty
                    .iter()
                    .map(|name| LeanString::from(name.as_str()))
                    .collect(),
            })
            .collect(),
    }))
}

fn resolve(
    target: &str,
    stated: &FieldOverride,
) -> crate::Result<crate::core::world::overrides::FieldRule> {
    use crate::core::world::overrides::{FieldRule, generator_named, generator_names};

    match stated {
        // A template is recognised by looking like one. Nothing else has to be
        // declared, and a generator name can never contain `{{`.
        FieldOverride::Named(named) if named.contains("{{") => {
            Ok(FieldRule::Template(LeanString::from(named.as_str())))
        }
        FieldOverride::Named(named) => generator_named(named).ok_or_else(|| {
            let nearest = generator_names()
                .iter()
                .map(|candidate| {
                    (
                        crate::core::levenshtein_distance(candidate, named),
                        candidate,
                    )
                })
                .filter(|(distance, _)| *distance <= 3)
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, candidate)| *candidate);
            match nearest {
                Some(nearest) => crate::mp_err!(
                    "`world.fields` for `{target}`: no generator called `{named}` — did you mean \
                     `{nearest}`?"
                ),
                None => crate::mp_err!(
                    "`world.fields` for `{target}`: no generator called `{named}`. Known: {}",
                    generator_names().join(", ")
                ),
            }
        }),
        FieldOverride::Shaped(ShapedOverride::Int(range)) => Ok(FieldRule::Number {
            float: false,
            min: range.min,
            max: range.max,
        }),
        FieldOverride::Shaped(ShapedOverride::Float(range)) => Ok(FieldRule::Number {
            float: true,
            min: range.min,
            max: range.max,
        }),
        FieldOverride::Shaped(ShapedOverride::OneOf(options)) => Ok(FieldRule::OneOf(
            options
                .iter()
                .map(|o| LeanString::from(o.as_str()))
                .collect(),
        )),
        FieldOverride::Shaped(ShapedOverride::Constant(value)) => {
            Ok(FieldRule::Constant(value.clone()))
        }
        FieldOverride::Shaped(ShapedOverride::Pattern(pattern)) => {
            Ok(FieldRule::Pattern(LeanString::from(pattern.as_str())))
        }
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl MockCollectionConfig {
    /// Parse from JSON string
    pub fn from_json(content: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(content)
    }

    /// Parse from YAML string
    pub fn from_yaml(content: &str) -> Result<Self, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(content)
    }

    /// Parse from file (supports JSON, YAML, HAR based on extension)
    ///
    /// HAR files are automatically converted to static mocks with exact URL matching.
    pub async fn from_file(path: impl Into<PathBuf>) -> Result<Self, crate::FerrimockError> {
        let path = path.into();
        let content = tokio::fs::read_to_string(&path).await?;

        // Determine format from extension
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| crate::mp_err!("File has no extension"))?;

        match extension {
            "json" => {
                // Auto-detect HAR files by checking for "log" top-level key
                if content.trim_start().starts_with(r#"{"log":"#) || content.contains(r#""log":"#) {
                    Self::from_har(&content).await
                } else {
                    Ok(Self::from_json(&content)?)
                }
            }
            "har" => Self::from_har(&content).await,
            "yaml" | "yml" => Ok(Self::from_yaml(&content)?),
            _ => Err(crate::mp_err!("Unsupported file format: {extension}")),
        }
    }

    /// Parse from HAR (HTTP Archive) file content
    ///
    /// Converts HAR entries to static mocks with exact URL matching.
    /// Use consolidator afterwards for pattern detection and optimization.
    pub async fn from_har(content: &str) -> Result<Self, crate::FerrimockError> {
        let har = serde_json::from_str(content)?;
        let loader = HarLoader::new();
        let mocks = loader.convert_har_to_mocks(har).await?;

        Ok(Self {
            name: Some("Mocks from HAR file".to_string()),
            description: Some(
                "Auto-converted from HAR file - all entries loaded as static mocks".to_string(),
            ),
            enabled: true,
            vars: None,
            mocks,
            world: None,
        })
    }

    /// Convert to mock definitions
    pub async fn into_mock_definitions(self) -> crate::Result<Vec<MockDefinition>> {
        self.into_mock_definitions_with_dir(None, None).await
    }

    /// Convert to mock definitions with config directory for resolving relative file paths
    /// and optional global vars to merge with collection-level and mock-level vars.
    ///
    /// A collection carrying `serve:` needs the world those mocks serve; use
    /// [`Self::into_mock_definitions_in`] for that. Without one, a `serve:`
    /// mock is an error rather than a silently dropped route.
    pub async fn into_mock_definitions_with_dir(
        self,
        config_dir: Option<&std::path::Path>,
        global_vars: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> crate::Result<Vec<MockDefinition>> {
        self.into_mock_definitions_in(config_dir, global_vars, None)
            .await
    }

    /// [`Self::into_mock_definitions_with_dir`] against a world, so `serve:`
    /// mocks can expand into the routes that serve it.
    pub async fn into_mock_definitions_in(
        self,
        config_dir: Option<&std::path::Path>,
        global_vars: Option<&serde_json::Map<String, serde_json::Value>>,
        world: Option<&std::sync::Arc<crate::core::World>>,
    ) -> crate::Result<Vec<MockDefinition>> {
        // Merge: global <- collection
        let collection_merged = merge_vars(global_vars, self.vars.as_ref());

        let mut definitions = Vec::new();
        for mut config in self.mocks {
            // Merge: collection_merged <- mock
            let final_vars = merge_vars(collection_merged.as_ref(), config.vars.as_ref());

            // Lifted before lowering: `serve` is a mode, not a body, so the
            // ordinary response path must not see it.
            let serve = config.serve.take();
            if let Some(serve) = &serve {
                config.check_serve_is_alone(serve)?;
                config.priority = super::serve::priority_for(config.priority);
            }

            let mut def = config.into_mock_definition_with_dir(config_dir).await?;
            def.vars.clone_from(&final_vars);

            match serve {
                None => definitions.push(def),
                Some(serve) => {
                    let world = world.ok_or_else(|| {
                        crate::mp_err!(
                            "mock `{}`: `serve:` needs the entity world, which only the \
                             registry's loader supplies",
                            def.id
                        )
                    })?;
                    for mut served in super::serve::expand(def, &serve, world)? {
                        served.vars.clone_from(&final_vars);
                        definitions.push(served);
                    }
                }
            }
        }
        Ok(definitions)
    }
}

fn default_enabled() -> bool {
    true
}

// These functions take references because serde's skip_serializing_if requires &T -> bool
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(v: &bool) -> bool {
    *v
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_priority(v: &u32) -> bool {
    *v == 100
}

/// Merge two optional variable maps. Lower-level (overlay) values shadow higher-level (base) values.
fn merge_vars(
    base: Option<&serde_json::Map<String, serde_json::Value>>,
    overlay: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(o)) => Some(o.clone()),
        (Some(b), Some(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            Some(merged)
        }
    }
}

/// Single mock configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MockConfig {
    /// Unique identifier
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub id: LeanString,

    /// Human-readable description of what this mock does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Priority for matching (higher = matched first)
    #[serde(
        default = "default_priority",
        skip_serializing_if = "is_default_priority"
    )]
    pub priority: u32,

    /// Enabled flag
    #[serde(default = "default_enabled", skip_serializing_if = "is_true")]
    pub enabled: bool,

    /// Optional scope for test isolation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub scope: Option<LeanString>,

    /// Mock-level variables that shadow collection-level and global vars
    /// Accessible in templates as {{ vars.key }}
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::HashMap<String, serde_json::Value>>")
    )]
    pub vars: Option<serde_json::Map<String, serde_json::Value>>,

    /// Flat match configuration (new syntax)
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub match_config: Option<MatchConfig>,

    /// Request transformations (implies passthrough/PatchUpstream mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestTransformConfig>,

    /// Response definition (FullMock)
    #[serde(
        rename = "response",
        alias = "return",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub response_config: Option<ResponseConfig>,

    /// Response patches applied to upstream responses (PatchUpstream mode)
    /// Cannot be combined with a full mock response (body/template/json/file/template_file)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<ResponsePatchesConfig>,

    /// Delay before responding (e.g., "100ms", "2s", "500us")
    /// Works in all modes: full mock, passthrough, and patch.
    /// When set alone (no response/patch/request), enables passthrough with delay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,

    /// Retire this mock after it matches once, so the next request for the
    /// same thing falls through to the mock behind it. Chaining several of
    /// these replays a recorded sequence in order: the endpoint answers
    /// differently each time, and the last mock (without `once`) answers from
    /// then on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once: bool,

    /// Simulate a dropped connection instead of answering: headers commit,
    /// then the body stream errors, so the caller sees a transport failure
    /// (`fetch` raises `TypeError`, curl reports an aborted transfer).
    /// Combines with `delay` to fail slowly. Exclusive with `response`,
    /// `patch`, `sse` and `ws`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_error: Option<bool>,

    /// Server-Sent Events playback (streaming mock; `response` may only
    /// carry extra headers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse: Option<super::streaming::SseConfig>,

    /// WebSocket behavior (streaming mock; `response` may only carry
    /// extra headers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ws: Option<super::streaming::WsConfig>,

    /// Serve the entity world over a protocol at this mock's URL.
    ///
    /// The sibling of `sse` and `ws`: not a response body, but a protocol
    /// behavior bound to a matched URL. `match` says *where* the API answers,
    /// `serve` says *which schema* answers there — a schema file has no way to
    /// say either. Exclusive with `response`, `patch`, `sse` and `ws`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serve: Option<ServeConfig>,
}

/// Which schema serves a mock's URL, and over which protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ServeConfig {
    /// `serve: graphql` — unambiguous while the world holds one schema of
    /// that protocol.
    Protocol(String),
    /// `serve: { protocol: graphql, schema: schemas/filestore.graphql }` — required
    /// once the world holds more than one.
    Explicit {
        protocol: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        /// Behaviour a real service has that a document does not describe —
        /// conditional requests, soft deletes, problem details, replica lag.
        /// Opt-in per mount, and forced off for replay.
        #[serde(default, skip_serializing_if = "Behaviour::is_none")]
        behaviour: Behaviour,
    },
}

impl ServeConfig {
    #[must_use]
    pub fn protocol(&self) -> &str {
        match self {
            Self::Protocol(protocol) | Self::Explicit { protocol, .. } => protocol,
        }
    }

    /// What this mount asked the serve layer to do beyond answering.
    #[must_use]
    pub const fn behaviour(&self) -> Behaviour {
        match self {
            Self::Protocol(_) => Behaviour::none(),
            Self::Explicit { behaviour, .. } => *behaviour,
        }
    }

    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        match self {
            Self::Protocol(_) | Self::Explicit { schema: None, .. } => None,
            Self::Explicit {
                schema: Some(schema),
                ..
            } => Some(schema),
        }
    }
}

/// Written out rather than derived so a constructed mock and a deserialized
/// one start from the same place: `derive(Default)` would give `priority: 0`
/// and `enabled: false`, contradicting the serde defaults above.
impl Default for MockConfig {
    fn default() -> Self {
        Self {
            id: LeanString::default(),
            description: None,
            priority: default_priority(),
            enabled: default_enabled(),
            once: false,
            scope: None,
            vars: None,
            match_config: None,
            request: None,
            response_config: None,
            patch: None,
            delay: None,
            network_error: None,
            sse: None,
            ws: None,
            serve: None,
        }
    }
}

impl MockConfig {
    /// `serve` is a mode like `sse` and `ws`, so it excludes the other ways a
    /// mock can answer. `response` survives for the same reason it does under
    /// `sse`: extra headers are still the mock's to set.
    fn check_serve_is_alone(&self, serve: &ServeConfig) -> crate::Result<()> {
        let conflict = if self.patch.is_some() {
            "patch"
        } else if self.sse.is_some() {
            "sse"
        } else if self.ws.is_some() {
            "ws"
        } else if self.network_error == Some(true) {
            "network_error"
        } else if self.request.is_some() {
            "request transforms"
        } else if self
            .response_config
            .as_ref()
            .is_some_and(super::response::ResponseConfig::is_full_mock)
        {
            "a full mock response body"
        } else {
            return Ok(());
        };

        Err(crate::mp_err!(
            "mock `{}`: cannot combine `serve: {}` with {conflict} — a served schema \
             produces the response, so there is nothing left to shape. To override part \
             of it, write a separate mock at a higher priority.",
            self.id,
            serve.protocol()
        ))
    }

    /// Convert to a MockDefinition
    pub async fn into_mock_definition(self) -> crate::Result<MockDefinition> {
        self.into_mock_definition_with_dir(None).await
    }

    /// Convert to a MockDefinition with config directory for resolving relative file paths
    pub async fn into_mock_definition_with_dir(
        self,
        config_dir: Option<&std::path::Path>,
    ) -> crate::Result<MockDefinition> {
        let match_config = self
            .match_config
            .ok_or_else(|| crate::mp_err!("Missing 'match' configuration"))?;

        let request_config = match_config.into_request_config();

        // Determine if we have request transforms
        let has_request_transforms = self.request.as_ref().is_some_and(|r| !r.is_empty());

        // Resolve the response config
        let network_error = self.network_error.unwrap_or(false);
        let response_config = if network_error {
            if self.response_config.is_some()
                || self.patch.is_some()
                || self.sse.is_some()
                || self.ws.is_some()
                || has_request_transforms
            {
                return Err(crate::mp_err!(
                    "`network_error` cannot be combined with `response`, `patch`, \
                     `sse`, `ws` or request transforms — the connection drops, so \
                     there is no response to shape"
                ));
            }
            // Desugars to the marker the server and the interceptor already
            // honour, so no runtime path is special-cased.
            Some(super::response::ResponseConfig::Structured {
                status: None,
                headers: rustc_hash::FxHashMap::default(),
                body: None,
                // The marker header is what tears the connection down, so the
                // status never reaches the wire.
                template: Some(format!(
                    r#"{{"headers": {{"{}": "1"}}, "body": ""}}"#,
                    crate::types::NETWORK_ERROR_HEADER
                )),
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            })
        } else {
            self.response_config
        };

        // Determine if this is a FullMock or PatchUpstream based on the heuristic:
        // - response.body or response.json set => FullMock
        // - response = 'template string' => FullMock
        // - response.NNN = "body" (status shortcut) => FullMock
        // - request.* set (any field) => PatchUpstream
        // - patch.* set => PatchUpstream
        // - Only response.status/delay/headers (no body/json) => FullMock
        let is_full_mock = response_config
            .as_ref()
            .is_some_and(super::response::ResponseConfig::is_full_mock);

        // Streaming mocks (ws/sse) are exclusive with body-producing and
        // upstream-patching modes; `response` may only carry status-less
        // extras (headers).
        let streaming = match (self.sse, self.ws) {
            (Some(_), Some(_)) => {
                return Err(crate::mp_err!("Cannot combine `sse` and `ws` in one mock"));
            }
            (Some(sse), None) => Some(crate::types::StreamingResponse::Sse(std::sync::Arc::new(
                sse.into_script()?,
            ))),
            (None, Some(ws)) => Some(crate::types::StreamingResponse::Ws(std::sync::Arc::new(
                ws.into_script()?,
            ))),
            (None, None) => None,
        };
        if streaming.is_some() {
            if is_full_mock {
                return Err(crate::mp_err!(
                    "Cannot combine `sse`/`ws` with a full mock response body; \
                     `response` may only set extra headers"
                ));
            }
            if self.patch.is_some() {
                return Err(crate::mp_err!("Cannot combine `sse`/`ws` with `patch`"));
            }
            if has_request_transforms {
                return Err(crate::mp_err!(
                    "Cannot combine `sse`/`ws` with request transforms"
                ));
            }
            if self.delay.is_some() {
                return Err(crate::mp_err!(
                    "Cannot combine `sse`/`ws` with a top-level `delay`; \
                     use per-event/per-action delays instead"
                ));
            }
        }

        // Validation: conflicting combinations
        if is_full_mock && has_request_transforms {
            return Err(
        "Cannot combine full mock body (response.body/response.json) with request transforms. \
         Use either a full mock OR passthrough with request transforms."
          .to_string().into(),
      );
        }

        // Validation: patch + full mock response is invalid
        if is_full_mock && self.patch.is_some() {
            return Err("Cannot combine top-level `patch` with full mock response. \
         Use either `patch` (upstream passthrough) or `response` (full mock), not both."
                .to_string()
                .into());
        }

        // Build the resolved response
        let resolved_response = response_config.unwrap_or_default().into_resolved_response();

        // Build response generator
        let mut response = resolved_response
            .into_response_generator_with_dir(config_dir)
            .await?;

        // Apply top-level delay
        if let Some(ref delay_str) = self.delay {
            let delay = parse_duration(delay_str)
                .map_err(|e| crate::mp_err!("Invalid top-level delay: {e}"))?;
            response = response.with_delay(delay);
        }

        // Apply top-level patches if configured
        if let Some(patches_config) = self.patch {
            let patch_ops = parse_patches_config(patches_config)?;
            if !patch_ops.is_empty() {
                response = response.with_mode(crate::types::ResponseMode::Patch {
                    operations: patch_ops,
                });
            }
        }

        // Delay-only passthrough: if top-level delay is set but no full mock body and no patches,
        // enter PatchUpstream mode with empty operations for upstream passthrough
        let entered_patch_mode = matches!(response.mode, crate::types::ResponseMode::Patch { .. });
        if self.delay.is_some() && !is_full_mock && !entered_patch_mode && !has_request_transforms {
            response = response.with_mode(crate::types::ResponseMode::Patch { operations: vec![] });
        }

        // Build request transforms if present
        let request_transforms = if has_request_transforms {
            let rt = self
                .request
                .ok_or_else(|| crate::mp_err!("request transforms missing"))?;
            Some(build_request_transforms(rt)?)
        } else {
            None
        };

        let mut request = request_config.into_request_matcher()?;
        if streaming
            .as_ref()
            .is_some_and(super::super::types::StreamingResponse::is_ws)
        {
            if request.methods.is_empty() {
                request.methods.push(http::Method::GET);
            } else if request.methods.iter().any(|m| m != http::Method::GET) {
                return Err(crate::mp_err!(
                    "`ws` mocks must match GET (the WebSocket handshake method)"
                ));
            }
            // Scope the mock to upgrade handshakes so plain GETs on the
            // same path fall through to other mocks.
            let upgrade_matcher =
                crate::types::HeaderMatcher::regex(http::header::UPGRADE, "(?i)^websocket$")
                    .map_err(|e| crate::mp_err!("internal upgrade matcher: {e}"))?;
            request.header_matchers.push(upgrade_matcher);
        }

        Ok(MockDefinition {
            id: self.id,
            priority: self.priority,
            enabled: self.enabled,
            once: self.once,
            scope: self.scope,
            source_file: None,
            request_transforms,
            request,
            response,
            vars: None,
            streaming,
        })
    }
}

/// Priority a mock takes when its config does not name one.
pub const DEFAULT_PRIORITY: u32 = 100;

const fn default_priority() -> u32 {
    DEFAULT_PRIORITY
}

/// Convert RequestTransformConfig into ResolvedRequestTransforms
fn build_request_transforms(
    config: RequestTransformConfig,
) -> crate::Result<crate::types::ResolvedRequestTransforms> {
    use crate::types::{RequestPatch, ResolvedRequestTransforms, UpstreamOptions};

    let mut patches = Vec::new();

    // Header patches
    for (name, value) in config.headers.add {
        patches.push(RequestPatch::HeaderAdd { name, value });
    }
    for name in config.headers.remove {
        patches.push(RequestPatch::HeaderRemove { name });
    }

    // Query patches
    for (name, value) in config.query.add {
        patches.push(RequestPatch::QueryAdd { name, value });
    }
    for name in config.query.remove {
        patches.push(RequestPatch::QueryRemove { name });
    }

    // Body patches - JSONPath
    for (path, value) in config.body.jsonpath {
        patches.push(RequestPatch::JsonPath { path, value });
    }

    // Body patches - RFC 6902
    if !config.body.operations.is_empty() {
        let json_patch_str = serde_json::to_string(&config.body.operations)
            .map_err(|e| crate::mp_err!("Failed to serialize JSON Patch operations: {e}"))?;
        let json_patch: json_patch::Patch = serde_json::from_str(&json_patch_str)
            .map_err(|e| crate::mp_err!("Failed to parse JSON Patch operations: {e}"))?;
        patches.push(RequestPatch::JsonPatch(json_patch));
    }

    // Body patches - Regex
    for regex_config in config.body.regex {
        let pattern = regex::Regex::new(&regex_config.pattern).map_err(|e| {
            crate::mp_err!("Invalid regex pattern '{}': {}", regex_config.pattern, e)
        })?;
        patches.push(RequestPatch::RegexReplace {
            pattern,
            replacement: regex_config.replacement,
        });
    }

    // Parse durations
    let pre_delay = config.delay.map(|d| parse_duration(&d)).transpose()?;

    let timeout = config.timeout.map(|t| parse_duration(&t)).transpose()?;

    Ok(ResolvedRequestTransforms {
        patches,
        pre_delay,
        upstream_options: UpstreamOptions {
            timeout,
            forward_to: config.forward_to,
        },
        rewrite_path: config.rewrite_path,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_mock_config() {
        let yaml = r#"
mocks:
  - id: test-mock
    priority: 100
    match:
      methods: ["GET"]
      url: /api/users
    response:
      status: 200
      body: '{"success": true}'
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.mocks.len(), 1);
        assert_eq!(config.mocks[0].id, "test-mock");
        assert_eq!(config.mocks[0].priority, 100);
    }

    #[tokio::test]
    async fn test_mock_config_with_headers() {
        let yaml = r#"
mocks:
  - id: test-mock
    match:
      methods: ["POST"]
      url: /api/users
      headers:
        content-type: application/json
    response:
      status: 201
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = config.mocks[0]
            .clone()
            .into_mock_definition()
            .await
            .expect("Failed to convert to mock definition");

        assert_eq!(mock_def.request.header_matchers.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_config_with_delay() {
        let yaml = r#"
mocks:
  - id: test-mock
    match:
      url: /api/users
    delay: 100ms
    response:
      status: 200
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = config.mocks[0]
            .clone()
            .into_mock_definition()
            .await
            .expect("Failed to convert to mock definition");

        assert_eq!(
            mock_def.response.delay,
            Some(std::time::Duration::from_millis(100))
        );
    }

    #[test]
    fn test_mock_collection_metadata() {
        let yaml = r#"
name: User API Mocks
description: Mock responses for user endpoints
enabled: true
mocks:
  - id: test-mock
    match:
      url: /test
    response:
      status: 200
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.name, Some("User API Mocks".to_string()));
        assert_eq!(
            config.description,
            Some("Mock responses for user endpoints".to_string())
        );
        assert!(config.enabled);
    }

    #[test]
    fn test_mock_config_default_priority() {
        let yaml = r#"
id: test
match:
  url: /test
response:
  body: "{}"
"#;

        let config: MockConfig =
            serde_yaml_ng::from_str(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.priority, 100);
    }

    #[test]
    fn test_mock_config_default_enabled() {
        let yaml = r#"
id: test
match:
  url: /test
response:
  body: "{}"
"#;

        let config: MockConfig =
            serde_yaml_ng::from_str(yaml).expect("Failed to parse YAML config");
        assert!(config.enabled);
    }

    #[tokio::test]
    async fn test_complete_mock_with_matchers_and_query() {
        let yaml = r#"
mocks:
  - id: advanced-mock
    priority: 100
    match:
      methods: ["POST"]
      url: /api/data
      body:
        "@important": true
      query:
        auth: "true"
    response:
      status: 200
      body: '{"success": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert!(mock_def[0].request.body_matcher.is_some());
        assert_eq!(mock_def[0].request.query_matchers.len(), 1);
    }

    // ============================================================================
    // Ultra-flat syntax tests
    // ============================================================================

    #[tokio::test]
    async fn test_ultra_flat_string_match() {
        let yaml = r#"
mocks:
  - id: ultra-flat
    match: "GET /api/users"
    response:
      status: 200
      body: '{"users": []}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].request.methods.len(), 1);
        assert_eq!(mock_def[0].request.methods[0], http::Method::GET);
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }

    #[tokio::test]
    async fn test_method_as_key_syntax() {
        let yaml = r#"
mocks:
  - id: method-key
    match:
      GET: /api/health
    response:
      status: 200
      body: '{"healthy": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].request.methods.len(), 1);
        assert_eq!(mock_def[0].request.methods[0], http::Method::GET);
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_method_shortcuts() {
        let yaml = r#"
mocks:
  - id: multi-methods
    match:
      POST: /api/users
      PUT: /api/users/:id
    response:
      status: 200
      body: "{}"
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        // Should have 2 methods and 2 URLs
        assert_eq!(mock_def[0].request.methods.len(), 2);
        assert!(mock_def[0].request.methods.contains(&http::Method::POST));
        assert!(mock_def[0].request.methods.contains(&http::Method::PUT));
        assert_eq!(mock_def[0].request.url_patterns.len(), 2);
    }

    #[tokio::test]
    async fn test_status_as_key_syntax() {
        let yaml = r#"
mocks:
  - id: status-key
    match: "GET /api/simple"
    response:
      "200": '{"success": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].response.status.as_u16(), 200);
    }

    #[tokio::test]
    async fn test_status_404_as_key() {
        let yaml = r#"
mocks:
  - id: not-found
    match: "GET /api/missing"
    response:
      "404": '{"error": "not found"}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        assert_eq!(mock_def[0].response.status.as_u16(), 404);
    }

    // ============================================================================
    // merge_vars tests
    // ============================================================================

    #[test]
    fn test_merge_vars_both_none() {
        assert!(merge_vars(None, None).is_none());
    }

    #[test]
    fn test_merge_vars_base_only() {
        let mut base = serde_json::Map::new();
        base.insert("key".to_string(), serde_json::json!("value"));
        let result = merge_vars(Some(&base), None);
        assert_eq!(result, Some(base));
    }

    #[test]
    fn test_merge_vars_overlay_only() {
        let mut overlay = serde_json::Map::new();
        overlay.insert("key".to_string(), serde_json::json!("value"));
        let result = merge_vars(None, Some(&overlay));
        assert_eq!(result, Some(overlay));
    }

    #[test]
    fn test_merge_vars_overlay_shadows() {
        let mut base = serde_json::Map::new();
        base.insert("color".to_string(), serde_json::json!("red"));
        let mut overlay = serde_json::Map::new();
        overlay.insert("color".to_string(), serde_json::json!("blue"));

        let result = merge_vars(Some(&base), Some(&overlay)).unwrap();
        assert_eq!(result.get("color").unwrap(), &serde_json::json!("blue"));
    }

    #[test]
    fn test_merge_vars_disjoint_keys() {
        let mut base = serde_json::Map::new();
        base.insert("a".to_string(), serde_json::json!(1));
        let mut overlay = serde_json::Map::new();
        overlay.insert("b".to_string(), serde_json::json!(2));

        let result = merge_vars(Some(&base), Some(&overlay)).unwrap();
        assert_eq!(result.get("a").unwrap(), &serde_json::json!(1));
        assert_eq!(result.get("b").unwrap(), &serde_json::json!(2));
    }

    #[test]
    fn test_collection_vars_parsed() {
        let yaml = r#"
vars:
  api_base: "https://api.example.com"
  version: 2
mocks:
  - id: test
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let vars = config.vars.unwrap();
        assert_eq!(
            vars.get("api_base").unwrap(),
            &serde_json::json!("https://api.example.com")
        );
        assert_eq!(vars.get("version").unwrap(), &serde_json::json!(2));
    }

    #[tokio::test]
    async fn test_mock_level_vars_shadow_collection() {
        let yaml = r#"
vars:
  color: red
  size: 10
mocks:
  - id: test
    vars:
      color: blue
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let defs = config
            .into_mock_definitions()
            .await
            .expect("Failed to convert");
        let vars = defs[0].vars.as_ref().unwrap();
        // Mock-level "color" should shadow collection-level
        assert_eq!(vars.get("color").unwrap(), &serde_json::json!("blue"));
        // Collection-level "size" should be inherited
        assert_eq!(vars.get("size").unwrap(), &serde_json::json!(10));
    }

    #[tokio::test]
    async fn test_global_vars_cascade() {
        let yaml = r#"
vars:
  from_collection: true
  shared: "collection"
mocks:
  - id: test
    vars:
      shared: "mock"
      from_mock: true
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let mut global_vars = serde_json::Map::new();
        global_vars.insert("from_global".to_string(), serde_json::json!(true));
        global_vars.insert("shared".to_string(), serde_json::json!("global"));

        let defs = config
            .into_mock_definitions_with_dir(None, Some(&global_vars))
            .await
            .expect("Failed to convert");
        let vars = defs[0].vars.as_ref().unwrap();

        // Mock-level wins for "shared"
        assert_eq!(vars.get("shared").unwrap(), &serde_json::json!("mock"));
        // Collection-level "from_collection" inherited
        assert_eq!(
            vars.get("from_collection").unwrap(),
            &serde_json::json!(true)
        );
        // Global-level "from_global" inherited
        assert_eq!(vars.get("from_global").unwrap(), &serde_json::json!(true));
        // Mock-level "from_mock" present
        assert_eq!(vars.get("from_mock").unwrap(), &serde_json::json!(true));
    }

    #[test]
    fn test_invalid_match_string_format() {
        let yaml = r#"
mocks:
  - id: invalid
    match: "GET"
    response:
      body: "{}"
"#;

        let result = serde_yaml_ng::from_str::<MockCollectionConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_http_method() {
        let yaml = r#"
mocks:
  - id: invalid
    match: "INVALID /api/test"
    response:
      body: "{}"
"#;

        let result = serde_yaml_ng::from_str::<MockCollectionConfig>(yaml);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_combined_ultra_flat_syntax() {
        let yaml = r#"
mocks:
  - id: combined
    match: "POST /api/users/:id"
    response:
      "201": '{"id": "{{ captures.id }}", "created": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        assert_eq!(mock_def[0].request.methods[0], http::Method::POST);
        assert_eq!(mock_def[0].response.status.as_u16(), 201);
        // URL pattern should be parsed as Express-style
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }
}
