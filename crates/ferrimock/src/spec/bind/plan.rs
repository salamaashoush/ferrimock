//! What an entry point *does*, independent of the protocol that exposes it.
//!
//! A GraphQL root field and an OpenAPI operation have nothing in common on the
//! wire, but they answer the same six questions: read one, read many, create,
//! update, delete, or none of those. Classifying each one once — into a rung
//! that is reportable — is what lets both front ends share a store and lets a
//! mock say how much of a spec it actually backs. The bottom rung invents data;
//! a mock that does that for half a surface must not look like one that does
//! not.
//!
//! The field names read as GraphQL because that is where they were written
//! first. Both readings are exact:
//!
//! | field           | GraphQL                    | REST                       |
//! |-----------------|----------------------------|----------------------------|
//! | `key_arg`       | field argument name        | path parameter name        |
//! | `input_arg`     | input argument name        | request body               |
//! | `payload_field` | field on a payload wrapper | field in a response envelope |
//! | `connection`    | Relay `ConnectionShape`    | pagination wrapper         |

use lean_string::LeanString;

use crate::core::world::model::ConnectionShape;

/// What an entry point resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootPlan {
    /// One instance, addressed by key.
    Get {
        entity: LeanString,
        /// Concrete entities behind an interface, union or polymorphic
        /// response. Empty when the entity is concrete.
        members: Vec<LeanString>,
        key_arg: LeanString,
    },
    /// Many instances, optionally wrapped in a connection or envelope.
    List {
        entity: LeanString,
        members: Vec<LeanString>,
        connection: Option<ConnectionShape>,
        /// The field holding the entities, when the list is wrapped in a
        /// result object rather than returned directly.
        payload_field: Option<LeanString>,
    },
    Create {
        entity: LeanString,
        input_arg: Option<LeanString>,
        payload_field: Option<LeanString>,
    },
    Update {
        entity: LeanString,
        key_arg: LeanString,
        input_arg: Option<LeanString>,
        payload_field: Option<LeanString>,
    },
    Delete {
        entity: LeanString,
        key_arg: LeanString,
        payload_field: Option<LeanString>,
    },
    /// Nothing about the entry point says what it does. Answered from its
    /// declared return shape, stably, and counted.
    Unclassified,
}

impl RootPlan {
    #[must_use]
    pub fn rung(&self) -> &'static str {
        match self {
            RootPlan::Get { .. } => "get",
            RootPlan::List { .. } => "list",
            RootPlan::Create { .. } => "create",
            RootPlan::Update { .. } => "update",
            RootPlan::Delete { .. } => "delete",
            RootPlan::Unclassified => "unclassified",
        }
    }

    #[must_use]
    pub fn is_classified(&self) -> bool {
        !matches!(self, RootPlan::Unclassified)
    }

    /// The entity this plan reads or writes, when it has one.
    #[must_use]
    pub fn entity(&self) -> Option<&LeanString> {
        match self {
            RootPlan::Get { entity, .. }
            | RootPlan::List { entity, .. }
            | RootPlan::Create { entity, .. }
            | RootPlan::Update { entity, .. }
            | RootPlan::Delete { entity, .. } => Some(entity),
            RootPlan::Unclassified => None,
        }
    }
}
