//! `ws.link(url | RegExp)` registration plus the native `client`/`server`
//! objects a connection listener drives (MSW WebSocket shapes).

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use rquickjs::function::{Func, Opt, This};
use rquickjs::{
    Class, Ctx, Exception, Function, JsLifetime, Object, Persistent, Value, class::Trace,
};
use tokio::sync::mpsc;

use crate::scripting::slots::{ScriptMockKind, ScriptMockSpec, with_slots};
use crate::streaming::WsUpstreamCmd;
use crate::types::{WsFrame, WsOutbound};

use super::streams::chunk_to_bytes;

fn value_to_frame<'js>(ctx: &Ctx<'js>, data: &Value<'js>) -> rquickjs::Result<WsFrame> {
    if let Some(s) = data.as_string() {
        return Ok(WsFrame::Text(s.to_string()?));
    }
    chunk_to_bytes(ctx, data).map(WsFrame::Binary)
}

/// The intercepted client connection (`connection.client`).
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "WebSocketClientConnection")]
pub struct WsClient {
    #[qjs(skip_trace)]
    conn_id: u64,
    #[qjs(skip_trace)]
    outbound: mpsc::UnboundedSender<WsOutbound>,
    #[qjs(skip_trace)]
    url: String,
}

impl WsClient {
    pub fn new(conn_id: u64, outbound: mpsc::UnboundedSender<WsOutbound>, url: String) -> Self {
        Self {
            conn_id,
            outbound,
            url,
        }
    }
}

#[rquickjs::methods]
impl WsClient {
    #[qjs(get)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    #[qjs(get)]
    pub fn id(&self) -> String {
        format!("ws:{:x}", self.conn_id)
    }

    /// Send text (string) or binary (TypedArray/ArrayBuffer) to the client.
    pub fn send<'js>(&self, ctx: Ctx<'js>, data: Value<'js>) -> rquickjs::Result<()> {
        let frame = value_to_frame(&ctx, &data)?;
        let _ = self.outbound.send(WsOutbound::Frame(frame));
        Ok(())
    }

    pub fn close(&self, ctx: Ctx<'_>, code: Opt<u16>, reason: Opt<String>) -> rquickjs::Result<()> {
        if let Some(code) = code.0
            && !(1000..=4999).contains(&code)
        {
            return Err(Exception::throw_range(
                &ctx,
                &format!("Invalid WebSocket close code: {code}"),
            ));
        }
        let _ = self.outbound.send(WsOutbound::Close {
            code: code.0,
            reason: reason.0,
        });
        Ok(())
    }

    /// `client.addEventListener('message' | 'close', listener)`.
    #[qjs(rename = "addEventListener")]
    pub fn add_event_listener<'js>(
        &self,
        ctx: Ctx<'js>,
        event: String,
        listener: Function<'js>,
    ) -> rquickjs::Result<()> {
        if event != "message" && event != "close" {
            return Err(Exception::throw_type(
                &ctx,
                "client events are 'message' and 'close'",
            ));
        }
        let persistent = Persistent::save(&ctx, listener);
        with_slots(&ctx, |slots| {
            slots.add_ws_connection_listener(self.conn_id, &event, persistent);
        })
    }
}

/// The real upstream connection (`connection.server`), idle until
/// `connect()` is called.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "WebSocketServerConnection")]
pub struct WsServer {
    #[qjs(skip_trace)]
    conn_id: u64,
    #[qjs(skip_trace)]
    cmd: mpsc::UnboundedSender<WsUpstreamCmd>,
    #[qjs(skip_trace)]
    url: Option<String>,
}

impl WsServer {
    pub fn new(
        conn_id: u64,
        cmd: mpsc::UnboundedSender<WsUpstreamCmd>,
        url: Option<String>,
    ) -> Self {
        Self { conn_id, cmd, url }
    }
}

#[rquickjs::methods]
impl WsServer {
    /// Dial the real server (the link's absolute URL) and start
    /// forwarding: upstream frames flow to the client unless a server
    /// `message` listener calls `event.preventDefault()`; client frames
    /// flow upstream unless a client `message` listener prevents it.
    pub fn connect(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        if cfg!(not(feature = "server")) {
            return Err(Exception::throw_type(
                &ctx,
                "ws server.connect() requires the `server` feature",
            ));
        }
        if self.url.is_none() {
            return Err(Exception::throw_type(
                &ctx,
                "ws server.connect() needs an absolute link URL (ws://host/path) to know the real server",
            ));
        }
        let _ = self.cmd.send(WsUpstreamCmd::Connect);
        Ok(())
    }

    /// Send to the real server.
    pub fn send<'js>(&self, ctx: Ctx<'js>, data: Value<'js>) -> rquickjs::Result<()> {
        let frame = value_to_frame(&ctx, &data)?;
        let _ = self.cmd.send(WsUpstreamCmd::Send(frame));
        Ok(())
    }

    pub fn close(&self) {
        let _ = self.cmd.send(WsUpstreamCmd::Close);
    }

    /// `server.addEventListener('open' | 'message' | 'error' | 'close', listener)`.
    #[qjs(rename = "addEventListener")]
    pub fn add_event_listener<'js>(
        &self,
        ctx: Ctx<'js>,
        event: String,
        listener: Function<'js>,
    ) -> rquickjs::Result<()> {
        if !matches!(event.as_str(), "open" | "message" | "error" | "close") {
            return Err(Exception::throw_type(
                &ctx,
                "server events are 'open', 'message', 'error', and 'close'",
            ));
        }
        let persistent = Persistent::save(&ctx, listener);
        let key = format!("server-{event}");
        with_slots(&ctx, |slots| {
            slots.add_ws_connection_listener(self.conn_id, &key, persistent);
        })
    }
}

fn link_add_event_listener<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    event: String,
    listener: Function<'js>,
) -> rquickjs::Result<()> {
    if event != "connection" {
        return Err(Exception::throw_type(
            &ctx,
            "ws links only emit the 'connection' event",
        ));
    }
    let link: Option<String> = this.get("__ferrimockWsLink")?;
    let Some(link) = link else {
        return Err(Exception::throw_type(
            &ctx,
            "ws.link handlers must be called on the link object (const chat = ws.link(url); chat.addEventListener(...))",
        ));
    };
    let link_id: u64 = link
        .parse()
        .map_err(|_| Exception::throw_type(&ctx, "corrupt ws link id"))?;

    let persistent = Persistent::save(&ctx, listener);
    with_slots(&ctx, |slots| {
        if slots.add_ws_link_listener(link_id, persistent)
            && let Some(path) = slots.ws_link_path(link_id)
        {
            slots.push_spec(ScriptMockSpec {
                kind: ScriptMockKind::Ws { path },
                slot: link_id,
                once: false,
                is_generator: false,
            });
        }
    })
}

fn ws_link<'js>(ctx: Ctx<'js>, url: Value<'js>) -> rquickjs::Result<Object<'js>> {
    let path = super::register::parse_path(url)?;
    let link_id = with_slots(&ctx, |slots| slots.new_ws_link(path))?;
    let link = Object::new(ctx)?;
    link.set("__ferrimockWsLink", link_id.to_string())?;
    link.set("addEventListener", Func::from(link_add_event_listener))?;
    Ok(link)
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Class::<WsClient>::define(&ctx.globals())?;
    Class::<WsServer>::define(&ctx.globals())?;
    let ws = Object::new(ctx.clone())?;
    ws.set("link", Func::from(ws_link))?;
    ctx.globals().set("ws", ws)?;
    Ok(())
}
