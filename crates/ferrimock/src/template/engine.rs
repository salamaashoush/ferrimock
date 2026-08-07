//! Template engine implementation with caching

use lru::LruCache;
use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use tera::{Context, Tera};

use super::functions::register_custom_functions;

/// Compute FxHash for a template string (public for pre-computation at load time)
pub fn hash_template(template: &str) -> u64 {
    TemplateEngine::hash_template(template)
}

/// LRU cache capacity for compiled templates per thread
const CACHE_CAPACITY: usize = 256;
/// Pre-computed NonZeroUsize for CACHE_CAPACITY (256 is non-zero)
const CACHE_CAPACITY_NZ: NonZeroUsize = match NonZeroUsize::new(CACHE_CAPACITY) {
    Some(v) => v,
    None => panic!("CACHE_CAPACITY must be non-zero"),
};

/// Every cached instance holds exactly one template, so they can all share a name.
const TEMPLATE_NAME: &str = "tpl";

thread_local! {
  /// Thread-local template engine instance (one per thread, reused across requests)
  /// Tera is not Sync, so we use thread_local! instead of static
  pub(super) static TEMPLATE_ENGINE: RefCell<TemplateEngine> = RefCell::new(TemplateEngine::new());

  /// Separate thread-local engine for template validation only.
  /// Kept separate from the render engine so validated templates don't pollute
  /// the render cache.
  pub(super) static VALIDATION_ENGINE: RefCell<ValidationEngine> = RefCell::new(ValidationEngine::new());
}

/// Build a Tera instance with every custom function registered and no templates.
///
/// Cloning one of these is ~45x cheaper than building it (0.6us vs 29us): the
/// registered closures are behind `Arc`, so a clone only duplicates the lookup
/// maps.
fn new_prototype() -> Tera {
    let mut tera = Tera::default();
    register_custom_functions(&mut tera);
    tera
}

/// Compile a template into its own instance, cloned from `prototype`.
fn compile(prototype: &Tera, template: &str) -> Result<Tera, tera::Error> {
    let mut tera = prototype.clone();
    tera.add_raw_template(TEMPLATE_NAME, template)?;
    Ok(tera)
}

/// Template engine with an LRU cache of compiled templates.
///
/// Each cache entry owns a Tera instance holding exactly one template, rather
/// than all templates sharing one instance. `Tera::add_raw_template` finalizes
/// the whole instance on every call - re-validating every template already in
/// it - so a shared instance makes compilation O(templates cached): 153us to
/// add the 256th template versus 6us into a fresh clone. Per-instance also
/// means eviction actually frees the compiled template, so there is no orphaned
/// template growth to periodically reset.
pub struct TemplateEngine {
    /// Registered functions, no templates. Cloned per compile.
    prototype: Tera,
    /// Compiled templates keyed by template hash (nohash for pre-hashed u64 keys)
    template_cache: LruCache<u64, Tera, BuildNoHashHasher<u64>>,
}

impl TemplateEngine {
    /// Create a new template engine with registered functions
    pub(super) fn new() -> Self {
        Self {
            prototype: new_prototype(),
            template_cache: LruCache::with_hasher(CACHE_CAPACITY_NZ, BuildNoHashHasher::default()),
        }
    }

    /// Render a template with caching (computes hash at call time)
    pub(super) fn render(
        &mut self,
        template: &str,
        tera_context: &Context,
    ) -> crate::Result<String> {
        let template_hash = Self::hash_template(template);
        self.render_with_hash(template, template_hash, tera_context)
    }

    /// Render a template with a pre-computed hash (skips hashing on the hot path)
    pub(super) fn render_with_hash(
        &mut self,
        template: &str,
        template_hash: u64,
        tera_context: &Context,
    ) -> crate::Result<String> {
        // get() rather than contains(), so a hit also updates LRU recency.
        if self.template_cache.get(&template_hash).is_none() {
            let compiled = compile(&self.prototype, template).map_err(|e| {
                let error = super::error::TemplateError::from_tera_error(&e, template);
                crate::FerrimockError::Template(format!("{error}"))
            })?;
            self.template_cache.put(template_hash, compiled);
        }

        let Some(tera) = self.template_cache.peek(&template_hash) else {
            return Err(crate::mp_err!(
                "internal error: template cache inconsistency"
            ));
        };

        tera.render(TEMPLATE_NAME, tera_context).map_err(|e| {
            let error = super::error::TemplateError::from_tera_error(&e, template);
            crate::FerrimockError::Template(error.to_string())
        })
    }

    /// Hash a template string for cache key
    pub fn hash_template(template: &str) -> u64 {
        let mut hasher = FxHasher::default();
        template.hash(&mut hasher);
        hasher.finish()
    }
}

/// Separate engine for template validation only.
///
/// Validation compiles into a throwaway clone of the prototype, so nothing
/// accumulates: bulk validation (e.g. MockValidator scanning files) costs the
/// same per template on the first file as on the thousandth.
pub struct ValidationEngine {
    prototype: Tera,
}

impl ValidationEngine {
    pub(super) fn new() -> Self {
        Self {
            prototype: new_prototype(),
        }
    }

    /// Validate a template by attempting to compile it
    #[allow(clippy::result_large_err)]
    pub(super) fn validate(&self, template: &str) -> Result<(), super::error::TemplateError> {
        compile(&self.prototype, template)
            .map(|_| ())
            .map_err(|e| super::error::TemplateError::from_tera_error(&e, template))
    }
}
