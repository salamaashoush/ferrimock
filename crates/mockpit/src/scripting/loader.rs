//! Module loading for script mock files.
//!
//! Files are bundled by rolldown (TS transpiled, `node_modules` and
//! relative imports inlined) before they reach the VM, so the runtime
//! loader chain only has to serve the `mockpit` native module — the one
//! import kept external so `import { http, HttpResponse } from
//! 'mockpit'` stays portable with the Node package.

use std::path::Path;

use rquickjs::loader::{BuiltinResolver, ModuleLoader};
use rquickjs::module::ModuleDef;
use rquickjs::{CatchResultExt, Ctx, Module, Value};

use crate::{MockpitError, Result, vm_with};

use super::bridge::caught_to_error;
use super::bundle::{CompiledBundle, bundle_and_compile, remap_error};
use super::engine::ScriptEngine;
use super::slots::{ScriptMockSpec, with_slots};

/// Bare specifier for the host-provided module.
pub const MOCKPIT_MODULE: &str = "mockpit";

/// Exposes the already-installed globals as module exports, so
/// `import { http } from 'mockpit'` observes the same bindings as
/// global access.
pub struct MockpitModule;

const MODULE_EXPORTS: [&str; 9] = [
    "http",
    "graphql",
    "HttpResponse",
    "fake",
    "delay",
    "passthrough",
    "bypass",
    "ws",
    "sse",
];

impl ModuleDef for MockpitModule {
    fn declare(decl: &rquickjs::module::Declarations<'_>) -> rquickjs::Result<()> {
        for name in MODULE_EXPORTS {
            decl.declare(name)?;
        }
        decl.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(
        ctx: &Ctx<'js>,
        exports: &rquickjs::module::Exports<'js>,
    ) -> rquickjs::Result<()> {
        let globals = ctx.globals();
        let default = rquickjs::Object::new(ctx.clone())?;
        for name in MODULE_EXPORTS {
            let value: Value<'js> = globals.get(name)?;
            exports.export(name, value.clone())?;
            default.set(name, value)?;
        }
        exports.export("default", default)?;
        Ok(())
    }
}

/// Resolver serving only the `mockpit` native module — everything else
/// was inlined by the bundler.
pub fn native_resolver() -> BuiltinResolver {
    BuiltinResolver::default().with_module(MOCKPIT_MODULE)
}

/// Loader counterpart of [`native_resolver`]. The built-in consuming
/// `ModuleLoader` is safe here: one engine hosts exactly one context,
/// so `mockpit` is only ever loaded once per loader instance.
pub fn native_loader() -> ModuleLoader {
    ModuleLoader::default().with_module(MOCKPIT_MODULE, MockpitModule)
}

/// Bundle + compile a mock script file, evaluate its bytecode on
/// `engine`'s VM, and drain the specs its `http.*`/`graphql.*` calls
/// registered. Returns the compiled bundle so the caller can keep the
/// source map for later error remapping.
pub async fn evaluate_mock_module(
    engine: &ScriptEngine,
    path: &Path,
    cwd: &Path,
) -> Result<(Vec<ScriptMockSpec>, CompiledBundle)> {
    let bundle = bundle_and_compile(path, cwd).await?;

    let vm = engine.vm().clone();
    let bytecode = std::sync::Arc::clone(&bundle.bytecode);
    let result: Result<Vec<ScriptMockSpec>> = vm_with!(vm => |ctx| {
        // SAFETY: produced by `Module::write` by this exact
        // rquickjs/QuickJS build with native endianness — either in this
        // process or restored from the bytecode disk cache, whose ABI
        // tag (QuickJS version, crate version, arch, endianness, pointer
        // width) + transitive input hashes guarantee an ABI-identical
        // toolchain wrote it. That satisfies the precondition
        // `Module::load` documents.
        #[allow(unsafe_code)]
        let module = match (unsafe { Module::load(ctx.clone(), &bytecode) }).catch(&ctx) {
            Ok(m) => m,
            Err(e) => return Err(caught_to_error(&e)),
        };
        let promise = match module.eval().catch(&ctx) {
            Ok((_, promise)) => promise,
            Err(e) => return Err(caught_to_error(&e)),
        };
        if let Err(e) = promise.into_future::<()>().await.catch(&ctx) {
            return Err(caught_to_error(&e));
        }
        with_slots(&ctx, super::slots::HandlerSlots::drain_specs)
            .map_err(|e| MockpitError::Script(e.to_string()))
    })
    .await?;

    match result {
        Ok(specs) => Ok((specs, bundle)),
        Err(e) => Err(remap_error(e, &bundle)),
    }
}
