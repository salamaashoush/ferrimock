//! `console.*` — forwarded to `tracing` under the `ferrimock::script`
//! target so script output lands in the host's log stream.

use rquickjs::function::{Func, Rest};
use rquickjs::{Ctx, Object, Value};

fn render_args<'js>(ctx: &Ctx<'js>, args: &[Value<'js>]) -> String {
    let mut parts = Vec::with_capacity(args.len());
    for value in args {
        if let Some(s) = value.as_string() {
            parts.push(s.to_string().unwrap_or_default());
        } else if let Ok(Some(json)) = ctx.json_stringify(value.clone()) {
            parts.push(json.to_string().unwrap_or_default());
        } else {
            parts.push(format!("{value:?}"));
        }
    }
    parts.join(" ")
}

macro_rules! console_fn {
    ($name:ident, $level:ident, $level_const:ident) => {
        fn $name<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) {
            // Skip the JSON-stringify render entirely when the level is off.
            if tracing::enabled!(target: "ferrimock::script", tracing::Level::$level_const) {
                tracing::$level!(target: "ferrimock::script", "{}", render_args(&ctx, &args.0));
            }
        }
    };
}

console_fn!(console_log, info, INFO);
console_fn!(console_info, info, INFO);
console_fn!(console_debug, debug, DEBUG);
console_fn!(console_warn, warn, WARN);
console_fn!(console_error, error, ERROR);

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let console = Object::new(ctx.clone())?;
    console.set("log", Func::from(console_log))?;
    console.set("info", Func::from(console_info))?;
    console.set("debug", Func::from(console_debug))?;
    console.set("warn", Func::from(console_warn))?;
    console.set("error", Func::from(console_error))?;
    ctx.globals().set("console", console)?;
    Ok(())
}
