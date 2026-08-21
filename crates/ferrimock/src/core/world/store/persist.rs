//! Keeping a world's writes across a restart.
//!
//! What gets written is the delta, not the entities. The base layer is pure —
//! a record is `(seed, entity, ordinal, field path)` — so a world of fifty
//! thousand records is a `u64` plus whatever anyone changed. Storing the
//! records themselves would mean materialising a world the census exists to
//! leave unbuilt, and would give the same world two sources of truth that can
//! disagree once a schema moves.
//!
//! So the file holds the seed it was taken against and the writes laid over
//! it. Reading it back is `import_delta`, which already reports the writes that
//! no longer fit rather than dropping them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::DeltaSnapshot;

/// A world's mutable state, and the seed it means something against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedWorld {
    /// The seed the writes were taken against. A file restored onto a
    /// different seed describes records that derive differently, so the
    /// mismatch is worth saying out loud rather than half-applying.
    pub seed: u64,
    #[serde(default)]
    pub delta: DeltaSnapshot,
}

/// Where a world's writes are kept, and the seed they belong to.
#[derive(Debug)]
pub struct Persistence {
    path: PathBuf,
    seed: u64,
}

impl Persistence {
    #[must_use]
    pub fn new(path: PathBuf, seed: u64) -> Self {
        Self { path, seed }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read what a previous run left, if anything did.
    ///
    /// A missing file is the ordinary first run, not a failure. A file that
    /// cannot be parsed is: it was written by this engine, so a caller that
    /// silently started empty would look identical to one that lost a day of
    /// state.
    pub fn load(&self) -> crate::Result<Option<PersistedWorld>> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(crate::mp_err!(
                    "reading world state from `{}`: {error}",
                    self.path.display()
                ));
            }
        };

        let held: PersistedWorld = serde_json::from_str(&raw).map_err(|error| {
            crate::mp_err!(
                "`{}` is not world state this engine wrote: {error}",
                self.path.display()
            )
        })?;

        if held.seed != self.seed {
            return Err(crate::mp_err!(
                "`{}` holds writes taken against seed {}, but this world is seeded {} — the \
                 records they name derive differently. Set `world.seed: {}` to restore them, or \
                 delete the file to start from the seed.",
                self.path.display(),
                held.seed,
                self.seed,
                held.seed
            ));
        }
        Ok(Some(held))
    }

    /// Write the delta out, atomically.
    ///
    /// Through a temporary beside the target and a rename, so a run killed
    /// mid-write leaves the previous state rather than half of this one.
    pub fn save(&self, delta: &DeltaSnapshot) -> crate::Result<()> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|error| crate::mp_err!("creating `{}`: {error}", parent.display()))?;
        }

        let held = PersistedWorld {
            seed: self.seed,
            delta: delta.clone(),
        };
        let body = serde_json::to_string_pretty(&held)
            .map_err(|error| crate::mp_err!("serializing world state: {error}"))?;

        let temporary = self.path.with_extension("tmp");
        std::fs::write(&temporary, body)
            .map_err(|error| crate::mp_err!("writing `{}`: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path).map_err(|error| {
            crate::mp_err!(
                "replacing `{}` with `{}`: {error}",
                self.path.display(),
                temporary.display()
            )
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::core::world::algebra::Mutation;
    use crate::core::world::model::{
        CompositeKey, EntityGraph, EntityType, FieldDef, Provenance, Rule, Scalar, ScalarKind,
        ValueSpec,
    };
    use crate::core::world::store::{EntityStore, StoreConfig};
    use std::sync::Arc;

    fn graph() -> Arc<EntityGraph> {
        let mut graph = EntityGraph::new();
        graph.insert(
            EntityType::new(
                "User",
                CompositeKey::single("id"),
                Provenance::new(Rule::Explicit, "test"),
            )
            .with_field(FieldDef::new(
                "id",
                ValueSpec::Scalar(Scalar::new(ScalarKind::Id)),
                false,
            ))
            .with_field(FieldDef::new(
                "name",
                ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
                false,
            )),
        );
        Arc::new(graph)
    }

    fn store(seed: u64, count: usize) -> EntityStore {
        let mut config = StoreConfig::seeded(seed);
        config.default_count = Some(count);
        EntityStore::new(graph(), config)
    }

    #[test]
    fn writes_survive_a_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let persist = Persistence::new(dir.path().join("state.json"), 42);

        let first = store(42, 0);
        first
            .apply(
                "User",
                Mutation::Insert {
                    values: serde_json::json!({ "name": "Ada" }),
                },
            )
            .unwrap();
        persist.save(&first.export_delta()).unwrap();

        // A second store built from the same seed knows nothing until it takes
        // the file back on.
        let second = store(42, 0);
        assert_eq!(second.count("User"), 0);
        let held = persist.load().unwrap().expect("a file was written");
        assert!(second.import_delta(held.delta).is_empty());

        assert_eq!(second.count("User"), 1);
        let key = second.keys("User").into_iter().next().unwrap();
        assert_eq!(
            second.get("User", &key).unwrap().get("name"),
            Some(&serde_json::json!("Ada"))
        );
    }

    #[test]
    fn a_first_run_finds_nothing_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let persist = Persistence::new(dir.path().join("absent.json"), 1);
        assert!(persist.load().unwrap().is_none());
    }

    /// Writes name records that derive from a seed. Restored onto a different
    /// one they describe a world that never existed, so the mismatch is refused
    /// by name rather than half-applied.
    #[test]
    fn state_taken_against_another_seed_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state.json");

        let written = store(42, 0);
        written
            .apply(
                "User",
                Mutation::Insert {
                    values: serde_json::json!({ "name": "Ada" }),
                },
            )
            .unwrap();
        Persistence::new(file.clone(), 42)
            .save(&written.export_delta())
            .unwrap();

        let error = Persistence::new(file, 7).load().unwrap_err().to_string();
        assert!(
            error.contains("42"),
            "the seed it was taken against: {error}"
        );
        assert!(error.contains('7'), "the seed it was loaded onto: {error}");
    }

    #[test]
    fn a_file_this_engine_did_not_write_is_an_error_not_an_empty_world() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state.json");
        std::fs::write(&file, "not json at all").unwrap();
        assert!(Persistence::new(file, 1).load().is_err());
    }
}
