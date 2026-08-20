//! Who is asking.
//!
//! A root field returning one instance with no way to say *which* — `viewer`,
//! `me`, `currentUser` — was answered with record zero, for every caller, with
//! or without a credential. That is not a small wrong answer: it is the one
//! endpoint whose whole purpose is to be different per caller.
//!
//! A viewer is a credential bound to an instance. The binding is derived, so
//! the same token is the same person on every request and across restarts, and
//! two tokens are two people without anything being stored.

use lean_string::LeanString;

use crate::core::world::model::EntityKey;
use crate::fake_data::rng;

/// Which entity a credential is an instance of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerBinding {
    pub entity: LeanString,
}

/// What a request said about who is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    /// A token, cookie or key. Its text is what identifies the caller; the
    /// scheme is not, because `Bearer abc` and `abc` are the same caller
    /// through two client libraries.
    Presented(String),
    /// Nothing was sent.
    Absent,
}

impl Credential {
    /// What the request carried, in the order a real service reads it.
    #[must_use]
    pub fn read(headers: &rustc_hash::FxHashMap<String, String>) -> Self {
        const CARRIERS: [&str; 4] = ["authorization", "x-api-key", "api-key", "cookie"];

        for name in CARRIERS {
            let held = headers
                .iter()
                .find(|(held, _)| held.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.trim());
            if let Some(value) = held.filter(|value| !value.is_empty()) {
                let bare = value.split_once(' ').map_or(value, |(_, rest)| rest.trim());
                return Self::Presented(bare.to_string());
            }
        }
        Self::Absent
    }

    /// Which instance this credential is, out of the keys an entity has.
    ///
    /// Derived rather than stored: the same token is the same person on every
    /// request and across a restart, and two tokens are two people, without a
    /// session table that would have to be kept.
    #[must_use]
    pub fn bound_to(&self, seed: u64, entity: &str, keys: &[EntityKey]) -> Option<EntityKey> {
        let Self::Presented(token) = self else {
            return None;
        };
        if keys.is_empty() {
            return None;
        }
        let stream = format!("{entity}#viewer:{token}");
        let drawn = rng::derive_seed(seed, &stream, 0);
        let at = usize::try_from(drawn % keys.len() as u64).ok()?;
        keys.get(at).cloned()
    }
}

/// What a service answers a request with no credential.
///
/// The header is not decoration: a client library that retries on 401 reads
/// the scheme out of it, and one that does not still logs it.
#[must_use]
pub const fn challenge() -> &'static str {
    "Bearer realm=\"ferrimock\""
}

#[cfg(test)]
mod tests;
