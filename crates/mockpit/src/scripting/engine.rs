//! The QuickJS script engine: runtime lifecycle, resource limits, and
//! the two-layer timeout guard for scripted handlers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rquickjs::{AsyncContext, AsyncRuntime};

use crate::error::Context as _;
use crate::{MockpitError, Result, vm_with};

use super::loader::{native_loader, native_resolver};
use super::vm::{VmHandle, VmShutdown, spawn_vm_loop};

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

const DISARMED: u64 = u64::MAX;

/// Shared deadline cell read by the QuickJS interrupt handler.
///
/// Rests at [`DISARMED`] between calls so a stale deadline can never
/// halt bytecode that runs outside an armed window. With concurrent
/// in-flight handlers the cell holds the earliest deadline; the guard is
/// a CPU-runaway backstop, not a precise per-call timer — the tokio
/// timeout in the handler bridge owns per-call latency.
pub struct TimeoutState {
    start: Instant,
    deadline_ms: AtomicU64,
    in_flight: AtomicUsize,
    pub timed_out: AtomicBool,
}

impl TimeoutState {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            deadline_ms: AtomicU64::new(DISARMED),
            in_flight: AtomicUsize::new(0),
            timed_out: AtomicBool::new(false),
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX - 1)
    }

    /// Register one in-flight call with `budget` from now; keeps the
    /// earliest active deadline.
    pub fn arm(&self, budget: Duration) {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        let deadline = self
            .now_ms()
            .saturating_add(u64::try_from(budget.as_millis()).unwrap_or(u64::MAX));
        self.deadline_ms.fetch_min(deadline, Ordering::Relaxed);
    }

    /// Unregister one in-flight call; disarms when none remain, else
    /// pushes the deadline out by `budget` for the survivors.
    pub fn disarm(&self, budget: Duration) {
        if self.in_flight.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.deadline_ms.store(DISARMED, Ordering::Relaxed);
        } else {
            let deadline = self
                .now_ms()
                .saturating_add(u64::try_from(budget.as_millis()).unwrap_or(u64::MAX));
            self.deadline_ms.store(deadline, Ordering::Relaxed);
        }
    }

    fn expired(&self) -> bool {
        self.now_ms() >= self.deadline_ms.load(Ordering::Relaxed)
    }
}

/// An embedded QuickJS engine hosting the handlers of one script file.
///
/// Owns one VM (runtime + context + event loop). All script evaluation
/// and every handler call goes through the engine's `VmHandle` jobs;
/// nothing else may touch the runtime.
pub struct ScriptEngine {
    _runtime: AsyncRuntime,
    vm: VmHandle,
    _shutdown: VmShutdown,
    timeout: Arc<TimeoutState>,
    poisoned: Arc<AtomicBool>,
    config: ScriptEngineConfig,
}

impl ScriptEngine {
    /// Build a runtime with limits + interrupt guard, wire the
    /// `mockpit` native-module loader, install the host bindings, and
    /// start the single-owner VM loop. All other imports were inlined
    /// by the bundler before code reaches this VM.
    pub async fn new(config: ScriptEngineConfig) -> Result<Self> {
        let runtime = AsyncRuntime::new()
            .map_err(|e| MockpitError::Script(format!("QuickJS runtime init: {e}")))?;
        runtime.set_memory_limit(config.memory_limit).await;
        runtime.set_max_stack_size(config.max_stack_size).await;
        runtime.set_gc_threshold(config.gc_threshold).await;

        let timeout = Arc::new(TimeoutState::new());
        {
            let state = Arc::clone(&timeout);
            runtime
                .set_interrupt_handler(Some(Box::new(move || {
                    if state.expired() {
                        state.timed_out.store(true, Ordering::Relaxed);
                        true
                    } else {
                        false
                    }
                })))
                .await;
        }

        runtime.set_loader(native_resolver(), native_loader()).await;

        let context = AsyncContext::full(&runtime)
            .await
            .context("QuickJS context init")?;
        let (vm, shutdown) = spawn_vm_loop(&context);

        vm_with!(vm => |ctx| {
            super::bindings::install_all(&ctx)
                .map_err(|e| MockpitError::Script(format!("install bindings: {e}")))
        })
        .await??;

        Ok(Self {
            _runtime: runtime,
            vm,
            _shutdown: shutdown,
            timeout,
            poisoned: Arc::new(AtomicBool::new(false)),
            config,
        })
    }

    /// Submission handle for VM jobs.
    pub(crate) fn vm(&self) -> &VmHandle {
        &self.vm
    }

    pub(crate) fn timeout_state(&self) -> &Arc<TimeoutState> {
        &self.timeout
    }

    pub fn config(&self) -> &ScriptEngineConfig {
        &self.config
    }

    /// A force-halt (interrupt fired) or OOM leaves the heap
    /// untrustworthy; the owner must discard this engine and reload its
    /// script file into a fresh one.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Relaxed)
    }

    pub(crate) fn poisoned_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.poisoned)
    }
}
