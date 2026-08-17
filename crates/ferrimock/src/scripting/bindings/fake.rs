//! `fake.*` — calls the fake data generator registry directly.
//!
//! Scripts see every generator (115+ built-ins plus anything an embedder added
//! via [`crate::template::register_template_function`]) because both this
//! binding and the Tera layer are consumers of the same registry.
//!
//! The JS surface is a `Proxy`: `fake.email()` forwards to the host as
//! `__ferrimock_fake("email", args)`, which resolves `fake_email` first
//! (the template naming convention) and the bare name second (`uuid`, plus
//! embedder extensions registered without the prefix).

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;

use rquickjs::function::{Func, Opt};
use rquickjs::{Ctx, Value};

use crate::template::fake_data::Args;

fn fake_call<'js>(
    ctx: Ctx<'js>,
    name: String,
    args: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    // The proxy forwards its `args` parameter verbatim, so a no-arg
    // `fake.uuid()` arrives as an explicit `undefined`.
    let args: Args = match args.0 {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            rquickjs_serde::from_value(v).map_err(|e| {
                rquickjs::Error::new_from_js_message("ferrimock", "TypeError", e.to_string())
            })?
        }
        _ => HashMap::new(),
    };

    let prefixed = format!("fake_{name}");
    let result = if let Some(generator) = crate::template::fake_data::generator(&prefixed)
        .or_else(|| crate::template::fake_data::generator(&name))
    {
        generator(&args)
    } else if let Some(plugin) = crate::template::plugin::lookup(&prefixed)
        .or_else(|| crate::template::plugin::lookup(&name))
    {
        plugin.call(&args)
    } else {
        return Err(rquickjs::Error::new_from_js_message(
            "ferrimock",
            "Error",
            format!("unknown fake data generator: {name}"),
        ));
    };

    let result = result.map_err(|e| {
        rquickjs::Error::new_from_js_message("ferrimock", "Error", format!("fake.{name}: {e}"))
    })?;

    // Not `rquickjs_serde::to_value`: a generator returning a number
    // (`fake.price()`, `fake.amount()`) would arrive in JS as the
    // `$serde_json::private::Number` map. See `super::convert`.
    super::convert::json_to_js(&ctx, &result)
}

const FAKE_PROXY: &str = r"
globalThis.fake = new Proxy({}, {
    get: (target, prop) => {
        if (typeof prop !== 'string') { return undefined; }
        return (args) => __ferrimock_fake(prop, args);
    },
});
";

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    ctx.globals()
        .set("__ferrimock_fake", Func::from(fake_call))?;
    ctx.eval::<(), _>(FAKE_PROXY)?;
    Ok(())
}
