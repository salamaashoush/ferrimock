//! VM-side storage for script handler functions and the mock specs a
//! script file registers while it evaluates.
//!
//! JS handler functions never leave the VM: `http.get(path, fn)` persists
//! the function here and records a [`ScriptMockSpec`] carrying its slot
//! id. The loader drains the specs after module evaluation and builds
//! [`crate::types::MockDefinition`]s whose bridge closures re-enter the
//! VM by slot id.
//!
//! Single-threaded VM ⇒ `RefCell`, never `Mutex`.

use std::cell::RefCell;

use rquickjs::{Ctx, Function, Object, Persistent, Value};
use rustc_hash::FxHashMap;

/// URL pattern a script registered: an Express-style/glob/exact string,
/// or a JS RegExp (carried as source + the flags relevant to matching).
#[derive(Debug, Clone)]
pub enum ScriptPath {
    Pattern(String),
    Regex {
        source: String,
        /// JS RegExp flags, filtered to the ones the regex crate can
        /// honor as inline flags (`i`, `m`, `s`).
        flags: String,
    },
}

/// GraphQL operation-name predicate: exact string or JS RegExp.
#[derive(Debug, Clone)]
pub enum ScriptGraphQLName {
    Exact(String),
    Regex { source: String, flags: String },
}

/// How a registered handler should be matched.
#[derive(Debug, Clone)]
pub enum ScriptMockKind {
    /// HTTP method (None = all methods) + path pattern.
    Http {
        method: Option<String>,
        path: ScriptPath,
    },
    /// GraphQL query/mutation by operation name, optionally scoped to an
    /// endpoint URL (`graphql.link`).
    GraphQL {
        operation_type: ScriptGraphQLOp,
        operation_name: Option<ScriptGraphQLName>,
        endpoint: Option<String>,
    },
    /// Server-Sent Events resolver (`sse(path, resolver)`).
    Sse { path: ScriptPath },
    /// WebSocket link (`ws.link(url | RegExp)`); the slot id is the link
    /// id whose connection listeners live in the link table.
    Ws { path: ScriptPath },
}

#[derive(Debug, Clone, Copy)]
pub enum ScriptGraphQLOp {
    Query,
    Mutation,
    Any,
}

/// One `http.*`/`graphql.*` registration made during module evaluation.
#[derive(Debug, Clone)]
pub struct ScriptMockSpec {
    pub kind: ScriptMockKind,
    pub slot: u64,
    pub once: bool,
    /// Resolver is a (async) generator function: each request advances
    /// its iterator; after exhaustion the last value repeats (MSW).
    pub is_generator: bool,
}

/// Per-connection WebSocket event listeners (client + upstream server).
#[derive(Default)]
pub struct WsConnListeners {
    pub message: Vec<Persistent<Function<'static>>>,
    pub close: Vec<Persistent<Function<'static>>>,
    pub server_open: Vec<Persistent<Function<'static>>>,
    pub server_message: Vec<Persistent<Function<'static>>>,
    pub server_error: Vec<Persistent<Function<'static>>>,
    pub server_close: Vec<Persistent<Function<'static>>>,
}

/// One `ws.link(...)` registration: its URL predicate plus the
/// connection listeners attached so far.
pub struct WsLink {
    pub path: ScriptPath,
    listeners: Vec<Persistent<Function<'static>>>,
}

#[derive(Default)]
pub struct HandlerSlots {
    handlers: FxHashMap<u64, Persistent<Function<'static>>>,
    /// Live iterator per generator-resolver slot (created on first request).
    iterators: FxHashMap<u64, Persistent<Object<'static>>>,
    /// Last non-undefined value a generator yielded (repeats after `done`).
    last_values: FxHashMap<u64, Persistent<Value<'static>>>,
    /// Connection listeners per `ws.link` (keyed by link slot id).
    ws_links: FxHashMap<u64, WsLink>,
    /// Client/server event listeners per live WebSocket connection.
    ws_connections: FxHashMap<u64, WsConnListeners>,
    /// Upstream EventSource listeners per live SSE connection, keyed by
    /// event type (`open`, `error`, or a frame's event name).
    sse_connections: FxHashMap<u64, FxHashMap<String, Vec<Persistent<Function<'static>>>>>,
    next_slot: u64,
    next_connection: u64,
    specs: Vec<ScriptMockSpec>,
}

impl HandlerSlots {
    pub fn insert(&mut self, handler: Persistent<Function<'static>>) -> u64 {
        let slot = self.next_slot;
        self.next_slot += 1;
        self.handlers.insert(slot, handler);
        slot
    }

    pub fn get(&self, slot: u64) -> Option<Persistent<Function<'static>>> {
        self.handlers.get(&slot).cloned()
    }

    pub fn iterator(&self, slot: u64) -> Option<Persistent<Object<'static>>> {
        self.iterators.get(&slot).cloned()
    }

    pub fn set_iterator(&mut self, slot: u64, iterator: Persistent<Object<'static>>) {
        self.iterators.insert(slot, iterator);
    }

    pub fn last_value(&self, slot: u64) -> Option<Persistent<Value<'static>>> {
        self.last_values.get(&slot).cloned()
    }

    pub fn set_last_value(&mut self, slot: u64, value: Persistent<Value<'static>>) {
        self.last_values.insert(slot, value);
    }

    pub fn push_spec(&mut self, spec: ScriptMockSpec) {
        self.specs.push(spec);
    }

    pub fn drain_specs(&mut self) -> Vec<ScriptMockSpec> {
        std::mem::take(&mut self.specs)
    }

    /// Allocate a `ws.link` id (shares the slot counter so link ids and
    /// handler slots never collide in specs).
    pub fn new_ws_link(&mut self, path: ScriptPath) -> u64 {
        let id = self.next_slot;
        self.next_slot += 1;
        self.ws_links.insert(
            id,
            WsLink {
                path,
                listeners: Vec::new(),
            },
        );
        id
    }

    pub fn ws_link_path(&self, link: u64) -> Option<ScriptPath> {
        self.ws_links.get(&link).map(|l| l.path.clone())
    }

    /// Returns true when this is the link's first connection listener
    /// (the caller registers the mock spec exactly once per link).
    pub fn add_ws_link_listener(
        &mut self,
        link: u64,
        listener: Persistent<Function<'static>>,
    ) -> bool {
        let Some(entry) = self.ws_links.get_mut(&link) else {
            return false;
        };
        entry.listeners.push(listener);
        entry.listeners.len() == 1
    }

    pub fn ws_link_listeners(&self, link: u64) -> Vec<Persistent<Function<'static>>> {
        self.ws_links
            .get(&link)
            .map(|l| l.listeners.clone())
            .unwrap_or_default()
    }

    pub fn new_ws_connection(&mut self) -> u64 {
        let id = self.next_connection;
        self.next_connection += 1;
        self.ws_connections.insert(id, WsConnListeners::default());
        id
    }

    pub fn add_ws_connection_listener(
        &mut self,
        connection: u64,
        event: &str,
        listener: Persistent<Function<'static>>,
    ) {
        if let Some(entry) = self.ws_connections.get_mut(&connection) {
            match event {
                "message" => entry.message.push(listener),
                "close" => entry.close.push(listener),
                "server-open" => entry.server_open.push(listener),
                "server-message" => entry.server_message.push(listener),
                "server-error" => entry.server_error.push(listener),
                "server-close" => entry.server_close.push(listener),
                _ => {}
            }
        }
    }

    pub fn ws_connection_listeners(
        &self,
        connection: u64,
        event: &str,
    ) -> Vec<Persistent<Function<'static>>> {
        self.ws_connections
            .get(&connection)
            .map(|entry| match event {
                "message" => entry.message.clone(),
                "close" => entry.close.clone(),
                "server-open" => entry.server_open.clone(),
                "server-message" => entry.server_message.clone(),
                "server-error" => entry.server_error.clone(),
                "server-close" => entry.server_close.clone(),
                _ => Vec::new(),
            })
            .unwrap_or_default()
    }

    /// Drop a closed connection's listeners (Persistent leak guard).
    pub fn remove_ws_connection(&mut self, connection: u64) {
        self.ws_connections.remove(&connection);
    }

    /// Allocate an SSE connection id (shared counter with WS connections).
    pub fn new_sse_connection(&mut self) -> u64 {
        let id = self.next_connection;
        self.next_connection += 1;
        self.sse_connections.insert(id, FxHashMap::default());
        id
    }

    /// Register a listener on an SSE connection's upstream source.
    /// `event` is `open`, `error`, or a frame's event name (`message`
    /// for unnamed frames) — EventSource dispatch semantics.
    pub fn add_sse_listener(
        &mut self,
        connection: u64,
        event: &str,
        listener: Persistent<Function<'static>>,
    ) {
        if let Some(entry) = self.sse_connections.get_mut(&connection) {
            entry.entry(event.to_string()).or_default().push(listener);
        }
    }

    pub fn sse_listeners(
        &self,
        connection: u64,
        event: &str,
    ) -> Vec<Persistent<Function<'static>>> {
        self.sse_connections
            .get(&connection)
            .and_then(|entry| entry.get(event).cloned())
            .unwrap_or_default()
    }

    /// Drop a closed SSE connection's listeners (Persistent leak guard).
    pub fn remove_sse_connection(&mut self, connection: u64) {
        self.sse_connections.remove(&connection);
    }
}

pub struct HandlerSlotsUd(RefCell<HandlerSlots>);

// SAFETY: holds only `'static` data (`Persistent<…>` handles), so
// re-stating the unused `'js` lifetime is sound.
#[allow(unsafe_code)]
unsafe impl rquickjs::JsLifetime<'_> for HandlerSlotsUd {
    type Changed<'to> = HandlerSlotsUd;
}

fn ensure_slots(ctx: &Ctx<'_>) {
    if ctx.userdata::<HandlerSlotsUd>().is_none() {
        let _ = ctx.store_userdata(HandlerSlotsUd(RefCell::new(HandlerSlots::default())));
    }
}

pub fn with_slots<R>(ctx: &Ctx<'_>, f: impl FnOnce(&mut HandlerSlots) -> R) -> rquickjs::Result<R> {
    ensure_slots(ctx);
    let ud = ctx.userdata::<HandlerSlotsUd>().ok_or_else(|| {
        rquickjs::Error::new_from_js_message(
            "ferrimock",
            "Error",
            "handler slots missing".to_string(),
        )
    })?;
    let mut slots = ud.0.borrow_mut();
    Ok(f(&mut slots))
}
