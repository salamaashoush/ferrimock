//! The script engine: one [`ferrijs::Runtime`] per mock file, built with
//! ferrimock's limits, its deny-all permission posture and the
//! `ferrimock` host module.
//!
//! Everything generic about running JavaScript (the VM event loop, the
//! interrupt deadline and its backstop, poison detection, the standard
//! globals, `require`, the bundler and its bytecode cache) is ferrijs's.
//! What this module adds is the MSW-shaped surface: the `ferrimock`
//! native module and the globals [`super::bindings::install_all`]
//! defines.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ferrijs::{
    ConsoleEntry, ConsoleLevel, ConsoleOptions, ConsoleSink, Extension, Identity, Limits,
    ModulePolicy, ModuleRegistry, Permissions, Runtime, ScriptError,
};
use ferrijs_bundle::{Bundler, BundlerOptions, BytecodeCache, CompiledModule};
use rquickjs::Ctx;

use crate::{FerrimockError, Result};

/// Resource limits and timeouts for the script engine.
#[derive(Debug, Clone)]
pub struct ScriptEngineConfig {
    /// Hard heap cap; exceeding it fails the current job with OOM.
    pub memory_limit: usize,
    /// Maximum interpreter stack size.
    pub max_stack_size: usize,
    /// Cycle-GC trigger threshold. Set high (LLRT-style) so short,
    /// object-churny handlers defer cycle collection; refcounting still
    /// frees acyclic garbage immediately and `memory_limit` stays the
    /// hard cap.
    pub gc_threshold: usize,
    /// Wall-clock budget for a single scripted handler call.
    pub handler_timeout: Duration,
}

impl Default for ScriptEngineConfig {
    fn default() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024,
            max_stack_size: 1024 * 1024,
            gc_threshold: 64 * 1024 * 1024,
            handler_timeout: Duration::from_secs(10),
        }
    }
}

impl ScriptEngineConfig {
    fn limits(&self) -> Limits {
        Limits {
            memory: self.memory_limit,
            stack: self.max_stack_size,
            gc_threshold: self.gc_threshold,
            timeout: self.handler_timeout,
            ..Limits::default()
        }
    }
}

/// `console.*` forwarded to `tracing` under the `ferrimock::script`
/// target, so script output lands in the host's log stream.
#[derive(Debug)]
struct ScriptConsole;

impl ConsoleSink for ScriptConsole {
    fn emit(&self, entry: &ConsoleEntry) {
        match entry.level {
            ConsoleLevel::Log | ConsoleLevel::Info => {
                tracing::info!(target: "ferrimock::script", "{}", entry.message);
            }
            ConsoleLevel::Debug | ConsoleLevel::Trace => {
                tracing::debug!(target: "ferrimock::script", "{}", entry.message);
            }
            ConsoleLevel::Warn | ConsoleLevel::System => {
                tracing::warn!(target: "ferrimock::script", "{}", entry.message);
            }
            ConsoleLevel::Error => {
                tracing::error!(target: "ferrimock::script", "{}", entry.message);
            }
        }
    }
}

/// The host surface of a mock realm: the `ferrimock` native module and
/// the MSW-shaped globals.
struct FerrimockExtension;

impl Extension for FerrimockExtension {
    fn name(&self) -> &'static str {
        "ferrimock"
    }

    fn modules(&self, registry: &mut ModuleRegistry) -> std::result::Result<(), String> {
        registry.register(super::loader::native_module())
    }

    fn install(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        super::bindings::install_all(ctx)
    }
}

/// Where compiled bytecode is kept between processes:
/// `$FERRIMOCK_CACHE_DIR/ferrimock`, else the platform user cache under
/// `ferrimock`; nowhere when `FERRIMOCK_NO_BYTECODE_CACHE` is set.
fn bytecode_cache() -> BytecodeCache {
    static CACHE: OnceLock<BytecodeCache> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            if std::env::var_os("FERRIMOCK_NO_BYTECODE_CACHE").is_some() {
                return BytecodeCache::disabled();
            }
            match std::env::var_os("FERRIMOCK_CACHE_DIR") {
                Some(dir) => BytecodeCache::at(PathBuf::from(dir).join("ferrimock")),
                None => BytecodeCache::for_app("ferrimock"),
            }
        })
        .clone()
}

/// An embedded runtime hosting the handlers of one script file.
///
/// Owns one realm (runtime + context + event loop). All script
/// evaluation and every handler call goes through the realm's run
/// bracket or its VM-loop handle; nothing else may touch the engine.
pub struct ScriptEngine {
    rt: Runtime,
    config: ScriptEngineConfig,
}

impl ScriptEngine {
    /// Build a realm with ferrimock's limits and posture: no filesystem,
    /// network, environment or system access, no `fetch`, no Node
    /// modules and no imports from disk. The `ferrimock` module is the
    /// only importable specifier; every other import was inlined by the
    /// bundler before code reaches this realm.
    pub async fn new(config: ScriptEngineConfig) -> Result<Self> {
        let rt = Runtime::builder()
            .limits(config.limits())
            .console(ConsoleOptions {
                sink: Some(Arc::new(ScriptConsole)),
                ..ConsoleOptions::default()
            })
            .permissions(Permissions::none())
            .without_fetch()
            .modules(ModulePolicy::default().no_builtins().no_files())
            .identity(Identity::new("ferrimock", env!("CARGO_PKG_VERSION")))
            .extension(FerrimockExtension)
            .build()
            .await?;
        Ok(Self { rt, config })
    }

    pub(crate) fn runtime(&self) -> &Runtime {
        &self.rt
    }

    /// The bundler for this realm's module table: only `ferrimock` stays
    /// external, everything else is inlined and compiled to bytecode
    /// through the disk cache.
    pub(crate) fn bundler(&self) -> Bundler {
        Bundler::new(
            BundlerOptions::default(),
            Arc::clone(self.rt.registry()),
            bytecode_cache(),
        )
    }

    pub fn config(&self) -> &ScriptEngineConfig {
        &self.config
    }

    /// A force-halt (interrupt fired), a run parked past its budget, or
    /// OOM leaves the heap untrustworthy; the owner must discard this
    /// engine and reload its script file into a fresh one.
    pub fn is_poisoned(&self) -> bool {
        self.rt.poisoned()
    }
}

/// A script failure as ferrimock reports it: the message, then the
/// stack (already mapped back to the original sources by the realm).
impl From<ScriptError> for FerrimockError {
    fn from(e: ScriptError) -> Self {
        match e.stack.as_deref().map(str::trim_end) {
            Some(stack) if !stack.is_empty() => Self::Script(format!("{}\n{stack}", e.message)),
            _ => Self::Script(e.message),
        }
    }
}

/// [`From<ScriptError>`] for a failure raised while `bundle` ran, with
/// the primary position (a syntax error's line, a thrown error's
/// innermost frame) translated back to the original `.ts`/`.js` file.
pub(super) fn remap_error(mut e: ScriptError, bundle: &CompiledModule) -> FerrimockError {
    if let Some(line) = e.line
        && let Some((src, sl, sc)) = bundle.remap(line, e.column.unwrap_or(1))
    {
        e.message = format!("{} (at {src}:{sl}:{sc})", e.message);
    }
    e.into()
}

pub(super) fn poisoned_error() -> FerrimockError {
    FerrimockError::Script(
        "script engine is poisoned (previous timeout/OOM); reload the script file".to_string(),
    )
}
