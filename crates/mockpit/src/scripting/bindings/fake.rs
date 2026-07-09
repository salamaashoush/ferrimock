//! `fake.*` — dispatches into the same function registry Tera templates
//! use, so scripts see every generator (115+ built-ins plus anything an
//! embedder added via
//! [`crate::template::register_template_function`]) with zero
//! duplication.
//!
//! The JS surface is a `Proxy`: `fake.email()` forwards to the host as
//! `__mockpit_fake("email", args)`, which resolves `fake_email` first
//! (the Tera naming convention) and the bare name second (`uuid`, plus
//! embedder extensions registered without the prefix).

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use rquickjs::function::{Func, Opt};
use rquickjs::{Ctx, Value};

/// Functions-only Tera instance (no templates ever rendered through it),
/// built once per process and shared across every engine. Embedder
/// functions must be registered before the first engine is created —
/// same constraint the per-engine snapshot had, minus the N-instances
/// memory cost.
static FAKE_TERA: OnceLock<Arc<tera::Tera>> = OnceLock::new();

pub struct FakeFunctionsUd(Arc<tera::Tera>);

// SAFETY: holds only `'static` data, so re-stating the unused `'js`
// lifetime is sound.
#[allow(unsafe_code)]
unsafe impl rquickjs::JsLifetime<'_> for FakeFunctionsUd {
    type Changed<'to> = FakeFunctionsUd;
}

fn fake_call<'js>(
    ctx: Ctx<'js>,
    name: String,
    args: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    // The proxy forwards its `args` parameter verbatim, so a no-arg
    // `fake.uuid()` arrives as an explicit `undefined`.
    let kwargs: HashMap<String, serde_json::Value> = match args.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            rquickjs_serde::from_value(v).map_err(|e| {
                rquickjs::Error::new_from_js_message("mockpit", "TypeError", e.to_string())
            })?
        }
        _ => HashMap::new(),
    };

    let ud = ctx.userdata::<FakeFunctionsUd>().ok_or_else(|| {
        rquickjs::Error::new_from_js_message(
            "mockpit",
            "Error",
            "fake data registry missing".to_string(),
        )
    })?;
    let tera = Arc::clone(&ud.0);
    drop(ud);

    let prefixed = format!("fake_{name}");
    let function = tera
        .get_function(&prefixed)
        .or_else(|_| tera.get_function(&name))
        .map_err(|_| {
            rquickjs::Error::new_from_js_message(
                "mockpit",
                "Error",
                format!("unknown fake data generator: {name}"),
            )
        })?;

    let result = function.call(&kwargs).map_err(|e| {
        rquickjs::Error::new_from_js_message("mockpit", "Error", format!("fake.{name}: {e}"))
    })?;
    rquickjs_serde::to_value(ctx, &result)
        .map_err(|e| rquickjs::Error::new_from_js_message("mockpit", "Error", e.to_string()))
}

const FAKE_PROXY: &str = r"
globalThis.fake = new Proxy({}, {
    get: (target, prop) => {
        if (typeof prop !== 'string') { return undefined; }
        return (args) => __mockpit_fake(prop, args);
    },
});
";

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    if ctx.userdata::<FakeFunctionsUd>().is_none() {
        let tera = Arc::clone(FAKE_TERA.get_or_init(|| {
            let mut tera = tera::Tera::default();
            crate::template::functions::register_custom_functions(&mut tera);
            Arc::new(tera)
        }));
        let _ = ctx.store_userdata(FakeFunctionsUd(tera));
    }
    ctx.globals().set("__mockpit_fake", Func::from(fake_call))?;
    ctx.eval::<(), _>(FAKE_PROXY)?;
    Ok(())
}
