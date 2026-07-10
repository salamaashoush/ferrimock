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

/// When total compiled templates exceed this multiple of the cache capacity,
/// reset the Tera instance to reclaim memory from orphaned (evicted) templates.
/// With a cache of 256, this triggers a reset after 512 unique templates.
const RESET_THRESHOLD_MULTIPLIER: usize = 2;

thread_local! {
  /// Thread-local template engine instance (one per thread, reused across requests)
  /// Tera is not Sync, so we use thread_local! instead of static
  pub(super) static TEMPLATE_ENGINE: RefCell<TemplateEngine> = RefCell::new(TemplateEngine::new());

  /// Separate thread-local Tera for template validation only.
  /// Kept separate from the render engine so validated templates don't pollute
  /// the render cache. Periodically reset to prevent unbounded memory growth.
  pub(super) static VALIDATION_ENGINE: RefCell<ValidationEngine> = RefCell::new(ValidationEngine::new());
}

/// Template engine with LRU cache for compiled templates
pub struct TemplateEngine {
    pub(super) tera: Tera,
    /// LRU cache mapping template hash to template ID (nohash for pre-hashed u64 keys)
    template_cache: LruCache<u64, String, BuildNoHashHasher<u64>>,
    /// Total number of templates ever compiled into this Tera instance.
    /// When this exceeds CACHE_CAPACITY * RESET_THRESHOLD_MULTIPLIER, we reset
    /// the Tera instance to free memory from orphaned (evicted) templates.
    total_compiled: usize,
}

impl TemplateEngine {
    /// Create a new template engine with registered functions
    pub(super) fn new() -> Self {
        Self {
            tera: Self::new_tera(),
            template_cache: LruCache::with_hasher(CACHE_CAPACITY_NZ, BuildNoHashHasher::default()),
            total_compiled: 0,
        }
    }

    /// Create a fresh Tera instance with all custom functions registered
    fn new_tera() -> Tera {
        let mut tera = Tera::default();
        register_custom_functions(&mut tera);
        tera
    }

    /// Reset the Tera instance to reclaim memory from orphaned templates.
    /// Clears the LRU cache so all templates get recompiled on next access.
    fn reset(&mut self) {
        self.tera = Self::new_tera();
        self.template_cache.clear();
        self.total_compiled = 0;
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
        // Check cache and compile if needed (get() updates LRU recency)
        if self.template_cache.get(&template_hash).is_none() {
            // Check if Tera has accumulated too many orphaned templates
            if self.total_compiled >= CACHE_CAPACITY * RESET_THRESHOLD_MULTIPLIER {
                self.reset();
            }

            // Cache miss - compile and cache the template
            let new_id = format!("tpl_{template_hash}");

            // Add the template to Tera
            self.tera.add_raw_template(&new_id, template).map_err(|e| {
                let error = super::error::TemplateError::from_tera_error(&e, template);
                crate::FerrimockError::Template(format!("{error}"))
            })?;

            // Store in cache and track total compiled
            self.template_cache.put(template_hash, new_id);
            self.total_compiled += 1;
        }

        // Use peek() to get a shared reference - avoids cloning the template ID.
        // The template was just inserted or confirmed present via get() above.
        let Some(template_id) = self.template_cache.peek(&template_hash) else {
            return Err(crate::mp_err!(
                "internal error: template cache inconsistency"
            ));
        };

        // Render the template
        self.tera.render(template_id, tera_context).map_err(|e| {
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

/// Maximum validations before resetting the validation Tera instance.
/// Keeps memory bounded during bulk validation (e.g., MockValidator scanning files).
const VALIDATION_RESET_THRESHOLD: usize = 500;

/// Separate engine for template validation only.
/// Unlike the render engine, this doesn't need caching (validation is not a hot path).
/// Periodically resets to prevent memory growth from accumulated validated templates.
pub struct ValidationEngine {
    tera: Tera,
    validation_count: usize,
}

impl ValidationEngine {
    pub(super) fn new() -> Self {
        Self {
            tera: TemplateEngine::new_tera(),
            validation_count: 0,
        }
    }

    /// Reset the Tera instance to reclaim memory from accumulated validated templates
    fn reset(&mut self) {
        self.tera = TemplateEngine::new_tera();
        self.validation_count = 0;
    }

    /// Validate a template by attempting to parse it
    #[allow(clippy::result_large_err)]
    pub(super) fn validate(&mut self, template: &str) -> Result<(), super::error::TemplateError> {
        let template_hash = TemplateEngine::hash_template(template);
        let template_id = format!("val_{template_hash}");

        match self.tera.add_raw_template(&template_id, template) {
            Ok(()) => {
                self.validation_count += 1;

                // Reset after threshold to prevent unbounded memory growth
                if self.validation_count >= VALIDATION_RESET_THRESHOLD {
                    self.reset();
                }

                Ok(())
            }
            Err(e) => Err(super::error::TemplateError::from_tera_error(&e, template)),
        }
    }
}
