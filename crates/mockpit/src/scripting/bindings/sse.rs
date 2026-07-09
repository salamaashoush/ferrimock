//! `sse(path, resolver)` registration plus the native `client`/`server`
//! objects a resolver drives (MSW's ServerSentEvent shapes).

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use rquickjs::function::{Func, Opt};
use rquickjs::{
    Class, Ctx, Exception, Function, JsLifetime, Object, Persistent, Value, class::Trace,
};
use tokio::sync::mpsc;

use crate::scripting::slots::{ScriptMockKind, ScriptMockSpec, with_slots};
use crate::streaming::SseUpstreamCmd;
use crate::types::{SseMessage, SseSinkMsg};

/// The `client` passed to an `sse()` resolver: pushes frames into the
/// live connection.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ServerSentEventClient")]
pub struct SseClient {
    #[qjs(skip_trace)]
    tx: mpsc::UnboundedSender<SseSinkMsg>,
}

impl SseClient {
    pub fn new(tx: mpsc::UnboundedSender<SseSinkMsg>) -> Self {
        Self { tx }
    }
}

#[rquickjs::methods]
impl SseClient {
    /// `client.send({ id?, event?, data?, retry? })` — objects in `data`
    /// are JSON-stringified (MSW semantics). Sends after close are
    /// silent no-ops.
    pub fn send<'js>(&self, ctx: Ctx<'js>, payload: Value<'js>) -> rquickjs::Result<()> {
        let Some(obj) = payload.as_object() else {
            return Err(Exception::throw_type(
                &ctx,
                "client.send expects an object payload ({ id?, event?, data?, retry? })",
            ));
        };
        let id: Option<String> = obj.get("id")?;
        let event: Option<String> = obj.get("event")?;
        let retry: Option<u32> = obj.get("retry")?;
        let data_value: Value<'js> = obj.get("data")?;
        let data = if data_value.is_undefined() || data_value.is_null() {
            String::new()
        } else if let Some(s) = data_value.as_string() {
            s.to_string()?
        } else {
            ctx.json_stringify(data_value)?
                .map(|s| s.to_string())
                .transpose()?
                .unwrap_or_default()
        };

        let _ = self.tx.send(SseSinkMsg::Message(SseMessage {
            id,
            event,
            data,
            retry,
        }));
        Ok(())
    }

    /// End the stream cleanly.
    pub fn close(&self) {
        let _ = self.tx.send(SseSinkMsg::Close);
    }

    /// Abort the connection (the consumer sees a network error).
    pub fn error(&self) {
        let _ = self.tx.send(SseSinkMsg::Error);
    }
}

/// The real upstream endpoint (`server` in the resolver info), idle
/// until `connect()` is called.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ServerSentEventServer")]
pub struct SseServer {
    #[qjs(skip_trace)]
    conn_id: u64,
    #[qjs(skip_trace)]
    cmd: mpsc::UnboundedSender<SseUpstreamCmd>,
    #[qjs(skip_trace)]
    url: Option<String>,
}

impl SseServer {
    pub fn new(
        conn_id: u64,
        cmd: mpsc::UnboundedSender<SseUpstreamCmd>,
        url: Option<String>,
    ) -> Self {
        Self { conn_id, cmd, url }
    }
}

#[rquickjs::methods]
impl SseServer {
    /// Open the real SSE connection (the handler's absolute http(s) URL)
    /// and start forwarding: upstream frames flow to the mocked client
    /// unless a listener on the returned source calls
    /// `event.preventDefault()`.
    pub fn connect<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, SseUpstreamSource>> {
        if cfg!(not(feature = "server")) {
            return Err(Exception::throw_type(
                &ctx,
                "sse server.connect() requires the `server` feature",
            ));
        }
        if self.url.is_none() {
            return Err(Exception::throw_type(
                &ctx,
                "sse server.connect() needs an absolute handler URL (http://host/path) to know the real endpoint",
            ));
        }
        let _ = self.cmd.send(SseUpstreamCmd::Connect);
        Class::instance(
            ctx,
            SseUpstreamSource {
                conn_id: self.conn_id,
                cmd: self.cmd.clone(),
            },
        )
    }
}

/// The EventSource-shaped handle `server.connect()` returns: listeners
/// receive the upstream frames before they forward to the client.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "SseUpstreamSource")]
pub struct SseUpstreamSource {
    #[qjs(skip_trace)]
    conn_id: u64,
    #[qjs(skip_trace)]
    cmd: mpsc::UnboundedSender<SseUpstreamCmd>,
}

#[rquickjs::methods]
impl SseUpstreamSource {
    /// `source.addEventListener('open' | 'error' | <event name>, listener)`
    /// — EventSource dispatch: named frames only reach listeners of that
    /// name, unnamed frames dispatch as `message`.
    #[qjs(rename = "addEventListener")]
    pub fn add_event_listener<'js>(
        &self,
        ctx: Ctx<'js>,
        event: String,
        listener: Function<'js>,
    ) -> rquickjs::Result<()> {
        let persistent = Persistent::save(&ctx, listener);
        with_slots(&ctx, |slots| {
            slots.add_sse_listener(self.conn_id, &event, persistent);
        })
    }

    /// Stop the upstream connection (terminal, like EventSource.close()).
    pub fn close(&self) {
        let _ = self.cmd.send(SseUpstreamCmd::Close);
    }
}

fn sse_register<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let path = super::register::parse_path(path)?;
    let once = super::register::parse_once(options.0)?;
    let persistent = Persistent::save(&ctx, handler);
    with_slots(&ctx, |slots| {
        let slot = slots.insert(persistent);
        slots.push_spec(ScriptMockSpec {
            kind: ScriptMockKind::Sse { path },
            slot,
            once,
            is_generator: false,
        });
    })
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Class::<SseClient>::define(&ctx.globals())?;
    Class::<SseServer>::define(&ctx.globals())?;
    Class::<SseUpstreamSource>::define(&ctx.globals())?;
    ctx.globals().set("sse", Func::from(sse_register))?;
    Ok(())
}
