//! `delay(ms | 'real' | 'infinite')` — MSW-compatible response delay.
//!
//! Returns a JS Promise backed by a tokio sleep; the future lives on the
//! VM scheduler, so a parked handler never blocks other dispatch jobs.
//! `'infinite'` never resolves — the per-call tokio backstop in the
//! handler bridge bounds how long the request itself hangs.

use std::time::Duration;

use rquickjs::function::{Func, Opt};
use rquickjs::promise::Promised;
use rquickjs::{Ctx, Value};

enum DelayMode {
    Exact(Duration),
    Real,
    Infinite,
}

fn parse_mode(value: Option<Value<'_>>) -> rquickjs::Result<DelayMode> {
    let Some(value) = value else {
        return Ok(DelayMode::Real);
    };
    if let Some(ms) = value.as_number() {
        let ms = if ms.is_finite() && ms >= 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                ms as u64
            }
        } else {
            0
        };
        return Ok(DelayMode::Exact(Duration::from_millis(ms)));
    }
    if let Some(s) = value.as_string() {
        return match s.to_string()?.as_str() {
            "real" => Ok(DelayMode::Real),
            "infinite" => Ok(DelayMode::Infinite),
            other => Err(rquickjs::Error::new_from_js_message(
                "ferrimock",
                "TypeError",
                format!("delay: expected a number, 'real', or 'infinite', got '{other}'"),
            )),
        };
    }
    if value.is_undefined() || value.is_null() {
        return Ok(DelayMode::Real);
    }
    Err(rquickjs::Error::new_from_js_message(
        "ferrimock",
        "TypeError",
        "delay: expected a number, 'real', or 'infinite'".to_string(),
    ))
}

fn delay(value: Opt<Value<'_>>) -> rquickjs::Result<Promised<impl Future<Output = ()> + use<>>> {
    let mode = parse_mode(value.0)?;
    Ok(Promised::from(async move {
        match mode {
            DelayMode::Exact(duration) => tokio::time::sleep(duration).await,
            // MSW's "realistic" server latency: random 100-400ms.
            DelayMode::Real => {
                let ms = 100 + rand::random::<u64>() % 301;
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            DelayMode::Infinite => std::future::pending::<()>().await,
        }
    }))
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    ctx.globals().set("delay", Func::from(delay))
}
