//! The `ferrimock` native module and mock-file evaluation.
//!
//! Files are bundled by rolldown (TS transpiled, `node_modules` and
//! relative imports inlined) before they reach the realm, so the module
//! table only has to serve `ferrimock` -- the one import kept external so
//! `import { http, HttpResponse } from 'ferrimock'` stays portable with
//! the Node package.

use std::sync::Arc;

use ferrijs::{NativeModule, RunOptions, ScriptError};
use ferrijs_bundle::CompiledModule;
use rquickjs::module::ModuleDef;
use rquickjs::{Ctx, Object, Value};

use crate::Result;

use super::engine::{ScriptEngine, remap_error};
use super::slots::{HandlerSlots, ScriptMockSpec, with_slots};

/// Bare specifier for the host-provided module.
pub const FERRIMOCK_MODULE: &str = "ferrimock";

/// Exposes the already-installed globals as module exports, so
/// `import { http } from 'ferrimock'` observes the same bindings as
/// global access.
pub struct FerrimockModule;

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

/// The exports as one object: the module's `default`, and what
/// `require('ferrimock')` hands back.
fn exports_object<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let globals = ctx.globals();
    let object = Object::new(ctx.clone())?;
    for name in MODULE_EXPORTS {
        let value: Value<'js> = globals.get(name)?;
        object.set(name, value)?;
    }
    Ok(object)
}

impl ModuleDef for FerrimockModule {
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
        let default = exports_object(ctx)?;
        for name in MODULE_EXPORTS {
            let value: Value<'js> = default.get(name)?;
            exports.export(name, value)?;
        }
        exports.export("default", default)?;
        Ok(())
    }
}

/// The `ferrimock` module as the realm (and the bundler) serve it.
pub fn native_module() -> NativeModule {
    NativeModule::new::<FerrimockModule, _>([FERRIMOCK_MODULE], exports_object)
}

/// Evaluate a compiled mock file on `engine`'s realm and drain the specs
/// its `http.*`/`graphql.*`/`sse`/`ws.link` calls registered. The
/// bundle's source map is registered on the realm first, so every later
/// failure (a handler throwing at request time included) reports the
/// original `.ts`/`.js` position.
pub async fn evaluate_mock_module(
    engine: &ScriptEngine,
    bundle: &CompiledModule,
) -> Result<Vec<ScriptMockSpec>> {
    let bytecode = Arc::clone(&bundle.bytecode);
    let mapper = bundle.mapper();
    let label = bundle.module_name.clone();
    let run = engine
        .runtime()
        .run(
            RunOptions::default(),
            Box::new(move |ctx| {
                Box::pin(async move {
                    ferrijs::source_map::register_bundle(&ctx, mapper);
                    ferrijs::eval_bytecode(&ctx, &bytecode, &label).await?;
                    with_slots(&ctx, HandlerSlots::drain_specs)
                        .map_err(|e| ScriptError::internal(e.to_string()))
                })
            }),
        )
        .await;
    run.result.map_err(|e| remap_error(e, bundle))
}
