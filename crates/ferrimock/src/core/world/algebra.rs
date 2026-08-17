//! What the store can be asked, in terms that mention no protocol.
//!
//! REST turns paths and query parameters into these; GraphQL turns root-field
//! arguments into them. Neither shape leaks into the store, which is the whole
//! reason one store serves both.

use lean_string::LeanString;
use serde_json::Value as JsonValue;

use super::model::EntityKey;

/// A read against one entity type.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub filters: Vec<Predicate>,
    pub sort: Vec<SortKey>,
    pub page: Page,
}

impl Selection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.filters.push(predicate);
        self
    }

    #[must_use]
    pub fn sorted_by(mut self, key: SortKey) -> Self {
        self.sort.push(key);
        self
    }

    #[must_use]
    pub fn paged(mut self, page: Page) -> Self {
        self.page = page;
        self
    }
}

/// A single filter condition. Field paths are dotted for embedded values
/// (`address.city`); relations are traversed with [`Selection`] instead.
#[derive(Debug, Clone)]
pub struct Predicate {
    pub field: LeanString,
    pub op: PredicateOp,
    pub value: JsonValue,
}

impl Predicate {
    #[must_use]
    pub fn eq(field: impl Into<LeanString>, value: JsonValue) -> Self {
        Self {
            field: field.into(),
            op: PredicateOp::Eq,
            value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateOp {
    Eq,
    Ne,
    In,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
}

#[derive(Debug, Clone)]
pub struct SortKey {
    pub field: LeanString,
    pub descending: bool,
}

impl SortKey {
    #[must_use]
    pub fn asc(field: impl Into<LeanString>) -> Self {
        Self {
            field: field.into(),
            descending: false,
        }
    }

    #[must_use]
    pub fn desc(field: impl Into<LeanString>) -> Self {
        Self {
            field: field.into(),
            descending: true,
        }
    }
}

/// Which slice of the result to return.
///
/// Cursors are opaque to callers and stable for a given seed and sort: the
/// binding hands back whatever the store produced and passes it in unchanged.
#[derive(Debug, Clone, Default)]
pub enum Page {
    #[default]
    All,
    Offset {
        skip: usize,
        take: usize,
    },
    After {
        cursor: Option<Cursor>,
        first: usize,
    },
    Before {
        cursor: Option<Cursor>,
        last: usize,
    },
}

/// A position in a sorted result set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor(LeanString);

impl Cursor {
    #[must_use]
    pub fn new(raw: impl Into<LeanString>) -> Self {
        Self(raw.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// A write against one entity type.
#[derive(Debug, Clone)]
pub enum Mutation {
    Insert {
        values: JsonValue,
    },
    /// Merge the given fields into an existing record.
    Patch {
        key: EntityKey,
        values: JsonValue,
    },
    /// Replace an existing record wholesale, keeping its key.
    Replace {
        key: EntityKey,
        values: JsonValue,
    },
    Remove {
        key: EntityKey,
    },
}
