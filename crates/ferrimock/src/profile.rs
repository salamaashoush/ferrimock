//! Domain knowledge the engine cannot have.
//!
//! Consolidation makes judgement calls that only an API's own conventions can
//! settle. Is `/api/2/users/1` a versioned endpoint or a tenant id? Is
//! `continuation` a cursor or an ordinary string field? Is
//! `https://files.example.com/...` a download link? The engine ships defensible
//! defaults, but the answers belong to whoever owns the API -- and that
//! knowledge is frequently not something its owner can publish.
//!
//! A [`ConsolidationProfile`] carries it. Pass one on
//! [`crate::consolidator::ConsolidatorOptions`] and it is consulted ahead of the
//! built-in heuristics; pass none and the built-ins are all there is.
//!
//! ```rust
//! use ferrimock::profile::{ConsolidationProfile, Placeholder, SegmentContext};
//!
//! struct MyApi;
//!
//! impl ConsolidationProfile for MyApi {
//!     fn name(&self) -> &str {
//!         "my-api"
//!     }
//!
//!     fn normalize_segment(&self, ctx: &SegmentContext<'_>) -> Option<Placeholder> {
//!         // `/2.1/` opens every path and is numeric enough to be mistaken for
//!         // an id, which would merge two API versions into one mock.
//!         (ctx.index == 1 && ctx.segment == "2.1").then_some(Placeholder::Literal)
//!     }
//!
//!     fn is_download_url(&self, url: &str) -> bool {
//!         url.contains("files.example.com")
//!     }
//! }
//! ```

// `ConsolidationProfile::name` returns `&str`, not `&'static str`, so a profile
// composed at runtime can name itself after its members. Implementations that
// answer with a literal look needlessly bound as a result.
#![allow(clippy::unnecessary_literal_bound)]

use crate::type_detector::FieldType;
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::sync::Arc;

/// One path segment, with everything known about where it sits.
///
/// `siblings` is what makes an informed answer possible: it holds the value seen
/// in this position for every recording in the group, so a profile can tell a
/// segment that varies from one that merely looks variable.
#[derive(Debug, Clone, Copy)]
pub struct SegmentContext<'a> {
    /// The segment being classified.
    pub segment: &'a str,
    /// Its position in the path, counting the empty segment before the leading
    /// slash as 0 -- so `/v2/files/1` has `v2` at index 1.
    pub index: usize,
    /// The segment before this one, if any. Usually the resource name.
    pub previous: Option<&'a str>,
    /// The segment after this one, if any.
    pub next: Option<&'a str>,
    /// The whole path the segment came from.
    pub path: &'a str,
    /// Every value seen in this position across the group being consolidated.
    /// A single distinct value means the segment did not vary.
    pub siblings: &'a [&'a str],
}

impl SegmentContext<'_> {
    /// How many different values this position took across the group.
    pub fn distinct_siblings(&self) -> usize {
        let mut seen: Vec<&str> = Vec::with_capacity(self.siblings.len());
        for sibling in self.siblings {
            if !seen.contains(sibling) {
                seen.push(sibling);
            }
        }
        seen.len()
    }

    /// Whether this position varied at all.
    pub fn varies(&self) -> bool {
        self.distinct_siblings() > 1
    }
}

/// The field names an API uses for paginating.
///
/// Consulted before the built-in names, so an API that calls its cursor
/// `continuation` is understood without the engine having to guess.
#[derive(Debug, Clone, Default)]
pub struct PaginationDialect {
    pub total: Vec<String>,
    pub offset: Vec<String>,
    pub limit: Vec<String>,
    pub next: Vec<String>,
    pub prev: Vec<String>,
    pub has_more: Vec<String>,
}

/// Domain knowledge consulted throughout consolidation.
///
/// Every method has a default that declines to answer, so a profile implements
/// only what it knows. Declining is not a failure -- it hands the decision back
/// to the built-in heuristics.
pub trait ConsolidationProfile: Send + Sync {
    /// Short identifier, used in diagnostics.
    fn name(&self) -> &str;

    /// Name the placeholder a path segment should collapse into, without braces
    /// -- returning `"file_id"` produces `{file_id}`.
    ///
    /// Return `None` to leave the segment to the built-in rules. Returning
    /// [`Placeholder::Literal`] keeps the segment as written, which is how a
    /// profile protects an API version from being read as an id.
    fn normalize_segment(&self, ctx: &SegmentContext<'_>) -> Option<Placeholder> {
        let _ = ctx;
        None
    }

    /// Type a field from domain knowledge, ahead of the built-in detector.
    ///
    /// The confidence is compared against the built-in detector's, so it has to
    /// mean the same thing: how likely this type is, not how strongly the
    /// profile would prefer it.
    fn classify_field(&self, field: &str, values: &[&JsonValue]) -> Option<(FieldType, f64)> {
        let _ = (field, values);
        None
    }

    /// Whether a URL serves file content.
    fn is_download_url(&self, url: &str) -> bool {
        let _ = url;
        false
    }

    /// The field names this API paginates with.
    fn pagination_dialect(&self) -> Option<&PaginationDialect> {
        None
    }

    /// A grouping discriminator drawn from the path, keeping unrelated resources
    /// apart even when their paths normalize alike.
    fn resource_key(&self, path: &str) -> Option<Cow<'_, str>> {
        let _ = path;
        None
    }

    /// Replace a value before it reaches a generated template or a report.
    ///
    /// Returning `Some` substitutes the value; returning `None` leaves it. This
    /// is how a recording of real traffic stops carrying credentials into a
    /// mock collection that gets committed.
    fn redact(&self, field: &str, value: &JsonValue) -> Option<JsonValue> {
        let _ = (field, value);
        None
    }
}

/// What a profile decided about a path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placeholder {
    /// Collapse the segment into `{name}`.
    Named(String),
    /// Keep the segment exactly as it is, whatever the built-in rules think.
    Literal,
}

impl Placeholder {
    /// Collapse into a named placeholder.
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }
}

/// The built-in behaviour, declining every domain question.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultProfile;

impl ConsolidationProfile for DefaultProfile {
    fn name(&self) -> &str {
        "default"
    }
}

/// Several profiles stacked, consulted in order.
///
/// The first profile to answer wins, so order encodes precedence -- a
/// domain profile ahead of a learned one keeps hand-written rules authoritative.
/// [`ConsolidationProfile::is_download_url`] is the exception: any profile
/// recognising the URL is enough.
pub struct CompositeProfile {
    name: String,
    profiles: Vec<Arc<dyn ConsolidationProfile>>,
}

impl CompositeProfile {
    pub fn new(profiles: Vec<Arc<dyn ConsolidationProfile>>) -> Self {
        let name = profiles
            .iter()
            .map(|profile| profile.name().to_string())
            .collect::<Vec<_>>()
            .join("+");
        Self { name, profiles }
    }
}

impl ConsolidationProfile for CompositeProfile {
    fn name(&self) -> &str {
        &self.name
    }

    fn normalize_segment(&self, ctx: &SegmentContext<'_>) -> Option<Placeholder> {
        self.profiles
            .iter()
            .find_map(|profile| profile.normalize_segment(ctx))
    }

    fn classify_field(&self, field: &str, values: &[&JsonValue]) -> Option<(FieldType, f64)> {
        self.profiles
            .iter()
            .find_map(|profile| profile.classify_field(field, values))
    }

    fn is_download_url(&self, url: &str) -> bool {
        self.profiles
            .iter()
            .any(|profile| profile.is_download_url(url))
    }

    fn pagination_dialect(&self) -> Option<&PaginationDialect> {
        self.profiles
            .iter()
            .find_map(|profile| profile.pagination_dialect())
    }

    fn resource_key(&self, path: &str) -> Option<Cow<'_, str>> {
        // The borrow cannot outlive the inner profile's own `self`, so the key
        // is taken by value rather than threaded back out by reference.
        self.profiles.iter().find_map(|profile| {
            profile
                .resource_key(path)
                .map(|key| key.into_owned().into())
        })
    }

    fn redact(&self, field: &str, value: &JsonValue) -> Option<JsonValue> {
        self.profiles
            .iter()
            .find_map(|profile| profile.redact(field, value))
    }

}

/// The profile used when a caller supplies none.
pub fn default_profile() -> Arc<dyn ConsolidationProfile> {
    Arc::new(DefaultProfile)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct Versions;
    impl ConsolidationProfile for Versions {
        fn name(&self) -> &str {
            "versions"
        }
        fn normalize_segment(&self, ctx: &SegmentContext<'_>) -> Option<Placeholder> {
            (ctx.segment == "2.0").then_some(Placeholder::Literal)
        }
        fn is_download_url(&self, url: &str) -> bool {
            url.contains("dl.example.com")
        }
    }

    struct Ids;
    impl ConsolidationProfile for Ids {
        fn name(&self) -> &str {
            "ids"
        }
        fn normalize_segment(&self, ctx: &SegmentContext<'_>) -> Option<Placeholder> {
            (ctx.segment.len() == 11).then(|| Placeholder::named("file_id"))
        }
        fn is_download_url(&self, url: &str) -> bool {
            url.contains("cdn.example.com")
        }
    }

    fn ctx<'a>(segment: &'a str, siblings: &'a [&'a str]) -> SegmentContext<'a> {
        SegmentContext {
            segment,
            index: 1,
            previous: None,
            next: None,
            path: "/",
            siblings,
        }
    }

    #[test]
    fn the_default_profile_declines_everything() {
        let profile = DefaultProfile;
        assert_eq!(profile.name(), "default");
        assert!(profile.normalize_segment(&ctx("2.0", &["2.0"])).is_none());
        assert!(profile.classify_field("id", &[]).is_none());
        assert!(!profile.is_download_url("https://dl.example.com/x"));
        assert!(profile.pagination_dialect().is_none());
        assert!(profile.resource_key("/x").is_none());
        assert!(profile.redact("token", &JsonValue::Null).is_none());
    }

    #[test]
    fn a_composite_asks_its_members_in_order() {
        let composite = CompositeProfile::new(vec![Arc::new(Versions), Arc::new(Ids)]);
        assert_eq!(composite.name(), "versions+ids");

        assert_eq!(
            composite.normalize_segment(&ctx("2.0", &["2.0"])),
            Some(Placeholder::Literal),
            "the first profile to answer wins"
        );
        assert_eq!(
            composite.normalize_segment(&ctx("12345678901", &["12345678901"])),
            Some(Placeholder::named("file_id")),
            "a question the first profile declines falls through"
        );
        assert!(
            composite
                .normalize_segment(&ctx("files", &["files"]))
                .is_none(),
            "a question nobody answers stays unanswered"
        );
    }

    #[test]
    fn any_member_recognising_a_download_url_is_enough() {
        let composite = CompositeProfile::new(vec![Arc::new(Versions), Arc::new(Ids)]);
        assert!(composite.is_download_url("https://dl.example.com/a"));
        assert!(composite.is_download_url("https://cdn.example.com/b"));
        assert!(!composite.is_download_url("https://api.example.com/c"));
    }

    #[test]
    fn sibling_values_say_whether_a_position_varied() {
        let varying = ctx("1", &["1", "2", "3"]);
        assert_eq!(varying.distinct_siblings(), 3);
        assert!(varying.varies());

        let constant = ctx("2.0", &["2.0", "2.0", "2.0"]);
        assert_eq!(constant.distinct_siblings(), 1);
        assert!(!constant.varies());

        let alone = ctx("files", &["files"]);
        assert!(!alone.varies());
    }
}
