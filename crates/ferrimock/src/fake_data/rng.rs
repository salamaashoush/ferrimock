//! Seedable random source shared by the fake-data generators, the template
//! functions and the scripting runtime.
//!
//! Unseeded, [`rng`] draws from OS entropy. Once a seed is set, output becomes
//! reproducible through two layers:
//!
//! - [`scope`] installs a stream derived from `(seed, stream name, ordinal)`
//!   for the duration of a render, so a mock renders the same bytes no matter
//!   which worker thread picks it up or how requests from other mocks
//!   interleave. Template rendering uses this, keyed by mock id.
//! - Outside any scope — a direct `fake.name()` call, a scripted handler —
//!   draws come from one process-wide stream, reproducible for a given call
//!   order.

use dashmap::DashMap;
use parking_lot::Mutex;
use rand::rngs::Xoshiro256PlusPlus;
use rand::{Rng as _, SeedableRng, TryRng};
use std::cell::RefCell;
use std::convert::Infallible;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Environment variable read once per process to set the global seed.
pub const SEED_ENV: &str = "FERRIMOCK_SEED";

static GLOBAL_SEED: AtomicU64 = AtomicU64::new(0);
static GLOBAL_SEED_SET: AtomicBool = AtomicBool::new(false);
static ENV_READ: OnceLock<()> = OnceLock::new();
static ORDINALS: OnceLock<DashMap<String, AtomicU64>> = OnceLock::new();
static GLOBAL_STREAM: Mutex<Option<Xoshiro256PlusPlus>> = Mutex::new(None);

thread_local! {
    static LOCAL: RefCell<Option<Xoshiro256PlusPlus>> = const { RefCell::new(None) };
}

fn ordinals() -> &'static DashMap<String, AtomicU64> {
    ORDINALS.get_or_init(DashMap::new)
}

fn read_env_seed() {
    ENV_READ.get_or_init(|| {
        if let Ok(raw) = std::env::var(SEED_ENV)
            && let Ok(seed) = raw.trim().parse::<u64>()
        {
            install_seed(seed);
        }
    });
}

fn install_seed(seed: u64) {
    GLOBAL_SEED.store(seed, Ordering::Relaxed);
    GLOBAL_SEED_SET.store(true, Ordering::Relaxed);
    *GLOBAL_STREAM.lock() = Some(Xoshiro256PlusPlus::seed_from_u64(derive_seed(
        seed, "<global>", 0,
    )));
}

/// Set (or clear, with `None`) the process-wide seed.
///
/// Either way every derived stream restarts, so a following
/// `set_global_seed(Some(n))` replays from the top.
pub fn set_global_seed(seed: Option<u64>) {
    // Mark the env as consumed so a later reader cannot overwrite an explicit
    // call with a stale `FERRIMOCK_SEED`.
    let _ = ENV_READ.set(());
    if let Some(value) = seed {
        install_seed(value);
    } else {
        GLOBAL_SEED_SET.store(false, Ordering::Relaxed);
        *GLOBAL_STREAM.lock() = None;
    }
    reset_streams();
}

/// The active process-wide seed, reading `FERRIMOCK_SEED` on first call.
#[must_use]
pub fn global_seed() -> Option<u64> {
    read_env_seed();
    GLOBAL_SEED_SET
        .load(Ordering::Relaxed)
        .then(|| GLOBAL_SEED.load(Ordering::Relaxed))
}

/// Whether generators are currently deterministic.
#[must_use]
pub fn is_seeded() -> bool {
    global_seed().is_some()
}

const fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Derive an independent stream seed. Stable across runs, platforms and
/// releases: FNV-1a over the stream name mixed into `SplitMix64`.
#[must_use]
pub fn derive_seed(seed: u64, stream: &str, ordinal: u64) -> u64 {
    derive_seed_parts(seed, &[stream], ordinal)
}

/// The same seed, from a stream name spelled in pieces.
///
/// FNV-1a folds one byte at a time, so hashing `["a", "#", "b"]` and hashing
/// `"a#b"` are the same arithmetic — which is the point. A stream name is
/// built per field of per record, and formatting one only to hash it and drop
/// it was the single hottest allocation in the store.
#[must_use]
pub fn derive_seed_parts(seed: u64, parts: &[&str], ordinal: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    splitmix64(seed ^ hash ^ splitmix64(ordinal))
}

/// Next ordinal for `stream`, counting from zero.
pub fn next_ordinal(stream: &str) -> u64 {
    if let Some(counter) = ordinals().get(stream) {
        return counter.fetch_add(1, Ordering::Relaxed);
    }
    ordinals()
        .entry(stream.to_owned())
        .or_insert_with(|| AtomicU64::new(0))
        .fetch_add(1, Ordering::Relaxed)
}

/// Restart every derived stream: the per-stream ordinal counters and the
/// process-wide stream used outside any [`scope`].
pub fn reset_streams() {
    ordinals().clear();
    if let Some(seed) = global_seed() {
        *GLOBAL_STREAM.lock() = Some(Xoshiro256PlusPlus::seed_from_u64(derive_seed(
            seed, "<global>", 0,
        )));
    }
}

/// Install a derived stream for the current thread until the guard drops.
///
/// A no-op (OS entropy, no ordinal consumed) when no seed is set.
#[must_use]
pub fn scope(stream: &str) -> SeedScope {
    match global_seed() {
        Some(seed) => SeedScope::install(derive_seed(seed, stream, next_ordinal(stream))),
        None => SeedScope::inactive(),
    }
}

/// Like [`scope`], but with a caller-chosen ordinal instead of the per-stream
/// counter — for replaying one exact draw.
#[must_use]
pub fn scope_at(stream: &str, ordinal: u64) -> SeedScope {
    match global_seed() {
        Some(seed) => SeedScope::install(derive_seed(seed, stream, ordinal)),
        None => SeedScope::inactive(),
    }
}

/// Install an explicit seed for the current thread, seeded or not.
#[must_use]
pub fn scope_seeded(seed: u64) -> SeedScope {
    SeedScope::install(seed)
}

/// Restores the previously installed thread-local stream when dropped.
#[derive(Debug)]
pub struct SeedScope {
    active: bool,
    previous: Option<Xoshiro256PlusPlus>,
}

impl SeedScope {
    const fn inactive() -> Self {
        Self {
            active: false,
            previous: None,
        }
    }

    fn install(seed: u64) -> Self {
        let previous =
            LOCAL.with_borrow_mut(|slot| slot.replace(Xoshiro256PlusPlus::seed_from_u64(seed)));
        Self {
            active: true,
            previous,
        }
    }
}

impl Drop for SeedScope {
    fn drop(&mut self) {
        if self.active {
            LOCAL.with_borrow_mut(|slot| *slot = self.previous.take());
        }
    }
}

// Each draw falls through the innermost source available: the thread's scoped
// stream, else the process-wide seeded stream, else OS entropy.

#[inline]
fn next_u32_impl() -> u32 {
    if let Some(value) =
        LOCAL.with_borrow_mut(|slot| slot.as_mut().map(Xoshiro256PlusPlus::next_u32))
    {
        return value;
    }
    if let Some(stream) = GLOBAL_STREAM.lock().as_mut() {
        return stream.next_u32();
    }
    rand::rng().next_u32()
}

#[inline]
fn next_u64_impl() -> u64 {
    if let Some(value) =
        LOCAL.with_borrow_mut(|slot| slot.as_mut().map(Xoshiro256PlusPlus::next_u64))
    {
        return value;
    }
    if let Some(stream) = GLOBAL_STREAM.lock().as_mut() {
        return stream.next_u64();
    }
    rand::rng().next_u64()
}

#[inline]
fn fill_bytes_impl(dst: &mut [u8]) {
    if LOCAL
        .with_borrow_mut(|slot| slot.as_mut().map(|rng| rng.fill_bytes(dst)))
        .is_some()
    {
        return;
    }
    if let Some(stream) = GLOBAL_STREAM.lock().as_mut() {
        stream.fill_bytes(dst);
        return;
    }
    rand::rng().fill_bytes(dst);
}

/// The random source every generator draws from.
///
/// Delegates to the thread's seeded stream when one is installed, otherwise to
/// `rand::rng()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FerriRng;

impl TryRng for FerriRng {
    type Error = Infallible;

    #[inline]
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(next_u32_impl())
    }

    #[inline]
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(next_u64_impl())
    }

    #[inline]
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        fill_bytes_impl(dst);
        Ok(())
    }
}

/// A version-4 UUID drawn from the current stream, so `--seed` covers every
/// generator that hands out identifiers.
#[must_use]
pub fn uuid_v4() -> uuid::Uuid {
    let mut bytes = [0_u8; 16];
    fill_bytes_impl(&mut bytes);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// Handle to the current random source. Cheap to construct; carries no state.
#[must_use]
#[inline]
pub const fn rng() -> FerriRng {
    FerriRng
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt;

    #[test]
    fn derive_seed_is_stable() {
        assert_eq!(
            derive_seed(42, "get-user", 0),
            derive_seed(42, "get-user", 0)
        );
        assert_ne!(
            derive_seed(42, "get-user", 0),
            derive_seed(42, "get-user", 1)
        );
        assert_ne!(
            derive_seed(42, "get-user", 0),
            derive_seed(42, "get-post", 0)
        );
        assert_ne!(
            derive_seed(42, "get-user", 0),
            derive_seed(43, "get-user", 0)
        );
    }

    #[test]
    fn scoped_streams_replay() {
        let draw = || {
            let _scope = scope_seeded(7);
            (0..4)
                .map(|_| rng().random_range(0..1000))
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(), draw());
    }

    #[test]
    fn scopes_nest_and_restore() {
        let _outer = scope_seeded(1);
        let first = rng().random::<u64>();

        {
            let _inner = scope_seeded(2);
            let _ = rng().random::<u64>();
        }

        let _replay = scope_seeded(1);
        assert_eq!(rng().random::<u64>(), first);
    }

    #[test]
    #[serial_test::serial]
    fn unseeded_scope_is_a_noop() {
        set_global_seed(None);
        let scope = scope("noop");
        assert!(!scope.active);
        assert_eq!(next_ordinal("noop"), 0);
        set_global_seed(None);
    }

    #[test]
    #[serial_test::serial]
    fn streams_are_independent_per_name() {
        set_global_seed(Some(5));
        let users = {
            let _scope = scope("get-user");
            super::super::fake_email()
        };

        set_global_seed(Some(5));
        // A different mock draws first this time; `get-user` must be unmoved.
        {
            let _other = scope("list-posts");
            let _ = super::super::fake_email();
        }
        let _scope = scope("get-user");
        assert_eq!(users, super::super::fake_email());
        set_global_seed(None);
    }

    // Two more properties belong here and cannot be asserted here: that a
    // global seed makes unscoped draws reproducible, and that `reset_streams`
    // restarts the process-wide stream. Both read a singleton, and this binary
    // runs ~180 other tests that draw from it without installing a scope — any
    // one of them landing between two draws moves the stream underneath the
    // assertion, and `#[serial]` only serialises against other serial tests.
    // They live in `tests/global_seed.rs`, which is its own process.
}
