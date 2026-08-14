//! Lineage from consolidated mocks back to the recordings they subsume.

use lean_string::LeanString;
use rustc_hash::FxHashMap;

/// Which original mocks each consolidated mock stands in for.
///
/// Consolidation is lossy: a group of recorded mocks collapses into one
/// templated mock whose id no longer says which recordings it answers for.
/// Fidelity checking needs that link to tell "this request matched the mock
/// built from its own recording" apart from "this request matched a stranger",
/// which is the failure mode an over-broad `{id}` pattern produces.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    subsumes: FxHashMap<LeanString, Vec<LeanString>>,
}

impl Provenance {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `consolidated` answers for every id in `origins`.
    pub fn record<I>(&mut self, consolidated: impl Into<LeanString>, origins: I)
    where
        I: IntoIterator,
        I::Item: Into<LeanString>,
    {
        self.subsumes
            .entry(consolidated.into())
            .or_default()
            .extend(origins.into_iter().map(Into::into));
    }

    /// Record that `id` stands only for itself.
    pub fn record_identity(&mut self, id: impl Into<LeanString> + Clone) {
        let id = id.into();
        self.record(id.clone(), [id]);
    }

    /// Ids the consolidated mock stands in for. Empty when the id is unknown,
    /// which callers must treat as "cannot prove lineage" rather than "no
    /// lineage" -- a collection that never went through the consolidator has no
    /// provenance at all.
    pub fn origins(&self, consolidated: &str) -> &[LeanString] {
        self.subsumes
            .get(consolidated)
            .map_or(&[][..], Vec::as_slice)
    }

    pub fn descends_from(&self, consolidated: &str, origin: &str) -> bool {
        self.origins(consolidated)
            .iter()
            .any(|o| o.as_str() == origin)
    }

    /// Every consolidated mock paired with the origins it subsumes.
    pub fn entries(&self) -> impl Iterator<Item = (&LeanString, &[LeanString])> {
        self.subsumes
            .iter()
            .map(|(consolidated, origins)| (consolidated, origins.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.subsumes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.subsumes.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn origins_of_unknown_id_are_empty() {
        let provenance = Provenance::new();
        assert!(provenance.origins("nope").is_empty());
        assert!(!provenance.descends_from("nope", "anything"));
    }

    #[test]
    fn identity_lineage_points_at_itself() {
        let mut provenance = Provenance::new();
        provenance.record_identity("rec-1");
        assert!(provenance.descends_from("rec-1", "rec-1"));
        assert!(!provenance.descends_from("rec-1", "rec-2"));
    }

    #[test]
    fn a_group_lineage_covers_every_member() {
        let mut provenance = Provenance::new();
        provenance.record("rec-1-smart-template", ["rec-1", "rec-2", "rec-3"]);

        for origin in ["rec-1", "rec-2", "rec-3"] {
            assert!(provenance.descends_from("rec-1-smart-template", origin));
        }
        assert!(!provenance.descends_from("rec-1-smart-template", "rec-9"));
        assert_eq!(provenance.len(), 1);
    }
}
