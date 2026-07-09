//! The library error type.
//!
//! Public APIs return [`MockpitError`] (and the [`Result`] alias) instead of
//! `anyhow::Error` or `String`, so consumers can match on failure modes.

use std::fmt;

/// Errors produced by the mockpit engine.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MockpitError {
    /// A generic message (carries context that doesn't fit a specific variant).
    #[error("{0}")]
    Message(String),
    /// Invalid mock/collection configuration.
    #[error("configuration error: {0}")]
    Config(String),
    /// Template parse/render failure.
    #[error("template error: {0}")]
    Template(String),
    /// Invalid URL/regex/glob pattern.
    #[error("invalid pattern: {0}")]
    Pattern(String),
    /// A referenced item was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// Script evaluation or scripted handler failure.
    #[error("script error: {0}")]
    Script(String),
    /// Filesystem I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// YAML (de)serialization failure.
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

impl MockpitError {
    /// Build a [`MockpitError::Message`] from anything `Display`.
    pub fn msg(m: impl fmt::Display) -> Self {
        MockpitError::Message(m.to_string())
    }
}

impl From<String> for MockpitError {
    fn from(s: String) -> Self {
        MockpitError::Message(s)
    }
}

impl From<&str> for MockpitError {
    fn from(s: &str) -> Self {
        MockpitError::Message(s.to_string())
    }
}

impl From<regex::Error> for MockpitError {
    fn from(e: regex::Error) -> Self {
        MockpitError::Pattern(e.to_string())
    }
}

// Foreign error types that flow through `?` — collapsed into a Message.
// (No blanket `From<E: Error>`: that conflicts with the reflexive `From<T> for T`
// since MockpitError itself implements Error.)
macro_rules! from_display {
    ($($t:ty),* $(,)?) => {
        $(impl From<$t> for MockpitError {
            fn from(e: $t) -> Self { MockpitError::Message(e.to_string()) }
        })*
    };
}
from_display!(
    http::Error,
    http::method::InvalidMethod,
    http::header::InvalidHeaderName,
    http::header::InvalidHeaderValue,
    http::status::InvalidStatusCode,
    std::string::FromUtf8Error,
    std::str::Utf8Error,
    std::num::ParseIntError,
    std::num::ParseFloatError,
    json_patch::PatchError,
);

/// Result alias defaulting to [`MockpitError`].
pub type Result<T, E = MockpitError> = std::result::Result<T, E>;

/// Attach context to a `Result`/`Option`, mirroring `anyhow::Context`, producing
/// a [`MockpitError`]. Lets existing `.context(..)` / `.with_context(..)` sites
/// keep working after the anyhow purge.
pub trait Context<T> {
    /// Wrap the error with a context message.
    fn context<C: fmt::Display>(self, ctx: C) -> Result<T>;
    /// Wrap the error with a lazily-computed context message.
    fn with_context<C: fmt::Display, F: FnOnce() -> C>(self, f: F) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context<C: fmt::Display>(self, ctx: C) -> Result<T> {
        self.map_err(|e| MockpitError::Message(format!("{ctx}: {e}")))
    }
    fn with_context<C: fmt::Display, F: FnOnce() -> C>(self, f: F) -> Result<T> {
        self.map_err(|e| MockpitError::Message(format!("{}: {e}", f())))
    }
}

impl<T> Context<T> for Option<T> {
    fn context<C: fmt::Display>(self, ctx: C) -> Result<T> {
        self.ok_or_else(|| MockpitError::Message(ctx.to_string()))
    }
    fn with_context<C: fmt::Display, F: FnOnce() -> C>(self, f: F) -> Result<T> {
        self.ok_or_else(|| MockpitError::Message(f().to_string()))
    }
}

/// Construct a [`MockpitError::Message`] (replaces `anyhow::anyhow!`).
/// Accepts `format!` syntax or a single `Display` expression.
#[macro_export]
macro_rules! mp_err {
    ($fmt:literal $($arg:tt)*) => { $crate::MockpitError::Message(format!($fmt $($arg)*)) };
    ($e:expr) => { $crate::MockpitError::msg($e) };
}

/// Return early with a [`MockpitError::Message`] (replaces `anyhow::bail!`).
#[macro_export]
macro_rules! mp_bail {
    ($fmt:literal $($arg:tt)*) => { return Err($crate::MockpitError::Message(format!($fmt $($arg)*))) };
    ($e:expr) => { return Err($crate::MockpitError::msg($e)) };
}

/// Return early unless a condition holds (replaces `anyhow::ensure!`).
#[macro_export]
macro_rules! mp_ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !($cond) {
            return Err($crate::MockpitError::Message(format!($($arg)*)));
        }
    };
}
