//! Native minimal `ReadableStream` so MSW-style streamed response bodies
//! (`new HttpResponse(stream)`) work on the embedded runtime.
//!
//! Supports the underlying-source API handlers actually use:
//! `new ReadableStream({ start(c), pull(c), cancel() })` with
//! `controller.enqueue(chunk)`, `controller.close()`, and
//! `controller.error(e)`. Chunks are strings, TypedArrays, or
//! ArrayBuffers. The handler bridge drains the stream after the resolver
//! settles: enqueue/close wake the parked drain future, so `start`
//! callbacks that enqueue asynchronously (e.g. after `delay()`) stream
//! correctly.

// rquickjs method targets must take FromJs params owned and the
// macro-injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use bytes::Bytes;
use rquickjs::function::Opt;
use rquickjs::{
    Class, Ctx, Exception, JsLifetime, Object, Persistent, TypedArray, Value, class::Trace,
};

#[derive(Default)]
pub struct StreamState {
    chunks: VecDeque<Bytes>,
    closed: bool,
    errored: Option<String>,
    /// Drain future parked waiting for enqueue/close/error.
    waker: Option<Waker>,
}

impl StreamState {
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

type SharedState = Rc<RefCell<StreamState>>;

pub(super) fn chunk_to_bytes(ctx: &Ctx<'_>, chunk: &Value<'_>) -> rquickjs::Result<Bytes> {
    if let Some(s) = chunk.as_string() {
        return Ok(Bytes::from(s.to_string()?));
    }
    if let Some(ab) = chunk
        .as_object()
        .and_then(|o| rquickjs::ArrayBuffer::from_object(o.clone()))
    {
        return Ok(Bytes::copy_from_slice(ab.as_bytes().unwrap_or_default()));
    }
    if let Ok(ta) = TypedArray::<u8>::from_value(chunk.clone()) {
        return Ok(Bytes::copy_from_slice(ta.as_bytes().unwrap_or_default()));
    }
    Err(Exception::throw_type(
        ctx,
        "ReadableStream chunks must be strings, ArrayBuffers, or TypedArrays",
    ))
}

/// The controller passed to `start`/`pull`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ReadableStreamDefaultController")]
pub struct StreamController {
    #[qjs(skip_trace)]
    state: SharedState,
}

#[rquickjs::methods]
impl StreamController {
    pub fn enqueue<'js>(&self, ctx: Ctx<'js>, chunk: Value<'js>) -> rquickjs::Result<()> {
        let bytes = chunk_to_bytes(&ctx, &chunk)?;
        let mut state = self.state.borrow_mut();
        if state.closed || state.errored.is_some() {
            return Err(Exception::throw_type(&ctx, "stream is not readable"));
        }
        state.chunks.push_back(bytes);
        state.wake();
        Ok(())
    }

    pub fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.closed = true;
        state.wake();
    }

    pub fn error(&self, reason: Opt<Value<'_>>) {
        let message = reason
            .0
            .and_then(|v| {
                v.as_string()
                    .and_then(|s| s.to_string().ok())
                    .or_else(|| Some(format!("{v:?}")))
            })
            .unwrap_or_else(|| "stream errored".to_string());
        let mut state = self.state.borrow_mut();
        state.errored = Some(message);
        state.wake();
    }

    /// How much more the queue wants; always positive here (unbounded).
    #[qjs(get, rename = "desiredSize")]
    #[allow(clippy::unused_self)]
    pub fn desired_size(&self) -> f64 {
        1.0
    }
}

/// Minimal ReadableStream: constructor + `locked`. Reading happens
/// host-side (the response drain); `getReader()` is not exposed.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "ReadableStream")]
pub struct ReadableStream {
    #[qjs(skip_trace)]
    state: SharedState,
    #[qjs(skip_trace)]
    pull: RefCell<Option<Persistent<rquickjs::Function<'static>>>>,
    #[qjs(skip_trace)]
    start_result: RefCell<Option<Persistent<Value<'static>>>>,
    #[qjs(skip_trace)]
    controller: RefCell<Option<Persistent<Value<'static>>>>,
}

#[rquickjs::methods]
impl ReadableStream {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, source: Opt<Object<'js>>) -> rquickjs::Result<Self> {
        let state: SharedState = Rc::default();

        let controller = Class::instance(
            ctx.clone(),
            StreamController {
                state: Rc::clone(&state),
            },
        )?;
        let controller_value = controller.as_value().clone();

        let mut pull = None;
        let mut start_result = None;
        if let Some(source) = source.0 {
            pull = source
                .get::<_, Option<rquickjs::Function<'js>>>("pull")?
                .map(|f| Persistent::save(&ctx, f));
            if let Some(start) = source.get::<_, Option<rquickjs::Function<'js>>>("start")? {
                // Per spec, start runs at construction. A returned promise
                // is awaited by the drain before pulling.
                let result: Value<'js> = start.call((controller_value.clone(),))?;
                if !result.is_undefined() {
                    start_result = Some(Persistent::save(&ctx, result));
                }
            }
        }

        Ok(Self {
            state,
            pull: RefCell::new(pull),
            start_result: RefCell::new(start_result),
            controller: RefCell::new(Some(Persistent::save(&ctx, controller_value))),
        })
    }

    // Host-side draining never locks the stream from JS's view
    // (&self keeps it an instance getter).
    #[qjs(get)]
    #[allow(clippy::unused_self)]
    pub fn locked(&self) -> bool {
        false
    }
}

/// Handle the drain loop uses; detached from the class borrow so JS
/// callbacks can re-enter the class while the drain holds it.
pub struct DrainHandle {
    pub state: SharedState,
    pub pull: Option<Persistent<rquickjs::Function<'static>>>,
    pub start_result: Option<Persistent<Value<'static>>>,
    pub controller: Option<Persistent<Value<'static>>>,
}

/// Extract a drain handle when the value is a native ReadableStream.
pub fn as_stream(value: &Value<'_>) -> Option<DrainHandle> {
    let obj = value.as_object()?;
    let stream = Class::<ReadableStream>::from_object(obj)?;
    let stream = stream.borrow();
    Some(DrainHandle {
        state: Rc::clone(&stream.state),
        pull: stream.pull.borrow().clone(),
        start_result: stream.start_result.borrow_mut().take(),
        controller: stream.controller.borrow().clone(),
    })
}

/// Future resolving when the stream has a chunk, closes, or errors.
pub struct StreamReady {
    state: SharedState,
}

impl StreamReady {
    pub fn new(state: &SharedState) -> Self {
        Self {
            state: Rc::clone(state),
        }
    }
}

impl Future for StreamReady {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.state.borrow_mut();
        if !state.chunks.is_empty() || state.closed || state.errored.is_some() {
            return Poll::Ready(());
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Take everything currently buffered plus terminal state.
pub struct DrainStep {
    pub chunks: Vec<Bytes>,
    pub closed: bool,
    pub errored: Option<String>,
}

pub fn drain_step(state: &SharedState) -> DrainStep {
    let mut state = state.borrow_mut();
    DrainStep {
        chunks: state.chunks.drain(..).collect(),
        closed: state.closed,
        errored: state.errored.clone(),
    }
}
