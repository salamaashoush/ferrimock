//! The `HttpResponse` class (MSW-compatible, constructible, with the
//! standard statics) and the JS-return-value → [`DynamicResponse`]
//! conversion.
//!
//! Builders construct the Rust response parts inside the host call — the
//! JS side only ever holds an opaque class instance, so extraction after
//! the handler resolves is a field move, not a serialization pass.
//!

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use bytes::Bytes;
use http::StatusCode;
use rquickjs::function::{Func, Opt};
use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, TypedArray, Value, class::Trace};
use rustc_hash::FxHashMap;

use crate::MockpitError;
use crate::types::DynamicResponse;

use super::form_data::{self, FormData};
use super::streams;

/// Response body: bytes, or a native ReadableStream drained by the
/// handler bridge after the resolver settles.
enum ResponseBody {
    Bytes(Bytes),
    Stream(rquickjs::Persistent<Value<'static>>),
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "HttpResponse")]
pub struct HttpResponse {
    #[qjs(skip_trace)]
    status: Option<u16>,
    #[qjs(skip_trace)]
    status_text: Option<String>,
    #[qjs(skip_trace)]
    headers: Option<FxHashMap<String, String>>,
    #[qjs(skip_trace)]
    body: std::cell::RefCell<ResponseBody>,
}

type ResponseInit = (
    Option<u16>,
    Option<String>,
    Option<FxHashMap<String, String>>,
);

/// `{ status?, statusText?, headers? }` options bag accepted by the
/// constructor and every static. `headers` may be a plain object, an
/// array of pairs, or a `Headers` instance.
fn parse_init<'js>(ctx: &Ctx<'js>, init: Option<Object<'js>>) -> rquickjs::Result<ResponseInit> {
    let Some(init) = init else {
        return Ok((None, None, None));
    };
    let status: Option<u16> = init.get::<_, Option<u16>>("status")?;
    let status_text: Option<String> = init.get::<_, Option<String>>("statusText")?;
    let headers = match init.get::<_, Option<Value<'js>>>("headers")? {
        Some(v) if !v.is_undefined() && !v.is_null() => {
            let entries = super::request::headers_from_init(ctx, v)?;
            let mut map = FxHashMap::default();
            for (name, value) in entries {
                match map.entry(name.clone()) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(value);
                    }
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        // Duplicates collapse into the single-valued wire
                        // map: Set-Cookie newline-joined (split back at the
                        // boundaries), everything else comma-joined.
                        let sep = if name == "set-cookie" { "\n" } else { ", " };
                        let merged = format!("{}{sep}{value}", e.get());
                        e.insert(merged);
                    }
                }
            }
            Some(map)
        }
        _ => None,
    };
    Ok((status, status_text, headers))
}

fn with_content_type(
    headers: Option<FxHashMap<String, String>>,
    content_type: &str,
) -> FxHashMap<String, String> {
    let mut headers = headers.unwrap_or_default();
    headers
        .entry("content-type".to_string())
        .or_insert_with(|| content_type.to_string());
    headers
}

fn value_to_bytes(data: &Value<'_>) -> Option<Bytes> {
    if let Some(ab) = data
        .as_object()
        .and_then(|o| rquickjs::ArrayBuffer::from_object(o.clone()))
    {
        return Some(Bytes::copy_from_slice(ab.as_bytes().unwrap_or_default()));
    }
    TypedArray::<u8>::from_value(data.clone())
        .ok()
        .map(|ta| Bytes::copy_from_slice(ta.as_bytes().unwrap_or_default()))
}

fn build(init: ResponseInit, default_content_type: Option<&str>, body: Bytes) -> HttpResponse {
    let (status, status_text, headers) = init;
    let headers = match default_content_type {
        Some(ct) => Some(with_content_type(headers, ct)),
        None => headers,
    };
    HttpResponse {
        status: Some(status.unwrap_or(200)),
        status_text,
        headers,
        body: std::cell::RefCell::new(ResponseBody::Bytes(body)),
    }
}

fn marked(marker: &str, status: Option<u16>) -> HttpResponse {
    let mut headers = FxHashMap::default();
    headers.insert(marker.to_string(), "1".to_string());
    HttpResponse {
        status,
        status_text: None,
        headers: Some(headers),
        body: std::cell::RefCell::new(ResponseBody::Bytes(Bytes::new())),
    }
}

#[rquickjs::methods]
impl HttpResponse {
    /// `new HttpResponse(body, init)` — no implied content type. Body may
    /// be a string, ArrayBuffer/TypedArray, ReadableStream, FormData, or
    /// null/undefined.
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        body: Opt<Value<'js>>,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        let body_value = match body.0 {
            None => None,
            Some(v) if v.is_undefined() || v.is_null() => None,
            Some(v) => Some(v),
        };

        let Some(v) = body_value else {
            return Ok(build(parse_init(&ctx, init.0)?, None, Bytes::new()));
        };

        if let Some(obj) = v.as_object() {
            if Class::<streams::ReadableStream>::from_object(obj).is_some() {
                let mut response = build(parse_init(&ctx, init.0)?, None, Bytes::new());
                response.body = std::cell::RefCell::new(ResponseBody::Stream(
                    rquickjs::Persistent::save(&ctx, v),
                ));
                return Ok(response);
            }
            if let Some(fd) = Class::<FormData>::from_object(obj) {
                return Ok(Self::form_data_response(
                    fd.borrow().snapshot(),
                    parse_init(&ctx, init.0)?,
                ));
            }
        }

        let bytes = if let Some(s) = v.as_string() {
            Bytes::from(s.to_string()?)
        } else if let Some(b) = value_to_bytes(&v) {
            b
        } else {
            return Err(Exception::throw_type(
                &ctx,
                "HttpResponse body must be a string, ArrayBuffer, Uint8Array, ReadableStream, or FormData",
            ));
        };
        Ok(build(parse_init(&ctx, init.0)?, None, bytes))
    }

    #[qjs(get)]
    pub fn status(&self) -> Option<u16> {
        self.status
    }

    #[qjs(get, rename = "statusText")]
    pub fn status_text(&self) -> Option<String> {
        self.status_text.clone()
    }

    #[qjs(get)]
    pub fn ok(&self) -> bool {
        self.status.is_some_and(|s| (200..300).contains(&s))
    }

    /// `"error"` for `HttpResponse.error()` results, `"default"` otherwise
    /// (the only two types a handler-built response can be).
    #[qjs(get, rename = "type")]
    pub fn response_type(&self) -> &'static str {
        let is_error = self
            .headers
            .as_ref()
            .is_some_and(|h| h.contains_key(crate::types::NETWORK_ERROR_HEADER));
        if is_error { "error" } else { "default" }
    }

    #[qjs(get)]
    pub fn headers<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let headers = match &self.headers {
            Some(map) => super::request::Headers::from_map(map),
            None => super::request::Headers::from_entries(Vec::new()),
        };
        Ok(Class::instance(ctx, headers)?.as_value().clone())
    }

    #[qjs(rename = "text")]
    pub fn body_text(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        let bytes = self.body_bytes(&ctx)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    #[qjs(rename = "json")]
    pub fn body_json<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let bytes = self.body_bytes(&ctx)?;
        ctx.json_parse(&bytes[..])
            .map_err(|_| Exception::throw_type(&ctx, "Failed to parse body as JSON"))
    }

    #[qjs(rename = "arrayBuffer")]
    pub fn body_array_buffer<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let bytes = self.body_bytes(&ctx)?;
        Ok(rquickjs::ArrayBuffer::new_copy(ctx, &bytes[..])?
            .as_value()
            .clone())
    }

    #[qjs(static)]
    pub fn json<'js>(
        ctx: Ctx<'js>,
        data: Value<'js>,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        let body = ctx
            .json_stringify(data)?
            .map(|s| s.to_string())
            .transpose()?
            .unwrap_or_else(|| "null".to_string());
        Ok(build(
            parse_init(&ctx, init.0)?,
            Some("application/json"),
            Bytes::from(body),
        ))
    }

    #[qjs(static)]
    pub fn text<'js>(
        ctx: Ctx<'js>,
        body: String,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        Ok(build(
            parse_init(&ctx, init.0)?,
            Some("text/plain"),
            Bytes::from(body),
        ))
    }

    #[qjs(static)]
    pub fn html<'js>(
        ctx: Ctx<'js>,
        body: String,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        Ok(build(
            parse_init(&ctx, init.0)?,
            Some("text/html"),
            Bytes::from(body),
        ))
    }

    #[qjs(static)]
    pub fn xml<'js>(ctx: Ctx<'js>, body: String, init: Opt<Object<'js>>) -> rquickjs::Result<Self> {
        Ok(build(
            parse_init(&ctx, init.0)?,
            Some("text/xml"),
            Bytes::from(body),
        ))
    }

    #[qjs(static, rename = "arrayBuffer")]
    pub fn array_buffer<'js>(
        ctx: Ctx<'js>,
        data: Value<'js>,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        let Some(bytes) = value_to_bytes(&data) else {
            return Err(Exception::throw_type(
                &ctx,
                "HttpResponse.arrayBuffer expects an ArrayBuffer or Uint8Array",
            ));
        };
        Ok(build(
            parse_init(&ctx, init.0)?,
            Some("application/octet-stream"),
            bytes,
        ))
    }

    #[qjs(static)]
    pub fn redirect(ctx: Ctx<'_>, url: String, status: Opt<u16>) -> rquickjs::Result<Self> {
        let status = status.0.unwrap_or(302);
        if !matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Err(Exception::throw_range(
                &ctx,
                &format!("Invalid redirect status code: {status}"),
            ));
        }
        let mut headers = FxHashMap::default();
        headers.insert("location".to_string(), url);
        Ok(build(
            (Some(status), None, Some(headers)),
            None,
            Bytes::new(),
        ))
    }

    /// Network error: status 0 + the network-error marker header. The
    /// Node interceptor throws `TypeError("Failed to fetch")`; the mock
    /// server aborts the connection.
    #[qjs(static)]
    pub fn error() -> Self {
        marked(crate::types::NETWORK_ERROR_HEADER, Some(0))
    }

    /// Multipart response from a `FormData` (MSW's `HttpResponse.formData`).
    #[qjs(static, rename = "formData")]
    pub fn form_data<'js>(
        ctx: Ctx<'js>,
        data: Value<'js>,
        init: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        let Some(fd) = data.as_object().and_then(Class::<FormData>::from_object) else {
            return Err(Exception::throw_type(
                &ctx,
                "HttpResponse.formData expects a FormData instance",
            ));
        };
        Ok(Self::form_data_response(
            fd.borrow().snapshot(),
            parse_init(&ctx, init.0)?,
        ))
    }
}

impl HttpResponse {
    fn body_bytes(&self, ctx: &Ctx<'_>) -> rquickjs::Result<Bytes> {
        match &*self.body.borrow() {
            ResponseBody::Bytes(bytes) => Ok(bytes.clone()),
            ResponseBody::Stream(_) => Err(Exception::throw_type(
                ctx,
                "Cannot read a ReadableStream body from the response instance",
            )),
        }
    }

    fn form_data_response(
        entries: Vec<(String, form_data::FormValue)>,
        init: ResponseInit,
    ) -> Self {
        let (body, boundary) = form_data::serialize_multipart(&entries);
        let (status, status_text, headers) = init;
        let mut headers = headers.unwrap_or_default();
        headers.insert(
            "content-type".to_string(),
            format!("multipart/form-data; boundary={boundary}"),
        );
        HttpResponse {
            status: Some(status.unwrap_or(200)),
            status_text,
            headers: Some(headers),
            body: std::cell::RefCell::new(ResponseBody::Bytes(body)),
        }
    }

    /// Split into wire metadata plus the stream to drain, if any. Byte
    /// bodies convert repeatedly (generator last-value repeats); a
    /// stream body moves out — it can only be drained once.
    fn to_converted(&self) -> ConvertedResponse {
        let meta = DynamicResponse {
            status: self.status.and_then(|code| StatusCode::from_u16(code).ok()),
            status_text: self.status_text.clone(),
            headers: self.headers.clone(),
            body: Bytes::new(),
        };
        if let ResponseBody::Bytes(bytes) = &*self.body.borrow() {
            return ConvertedResponse::Ready(DynamicResponse {
                body: bytes.clone(),
                ..meta
            });
        }
        match self.body.replace(ResponseBody::Bytes(Bytes::new())) {
            ResponseBody::Stream(stream) => ConvertedResponse::Streaming { meta, stream },
            ResponseBody::Bytes(bytes) => ConvertedResponse::Ready(DynamicResponse {
                body: bytes,
                ..meta
            }),
        }
    }
}

/// Outcome of converting a resolver's return value: either a complete
/// response, or metadata plus a ReadableStream the bridge must drain.
pub enum ConvertedResponse {
    Ready(DynamicResponse),
    Streaming {
        meta: DynamicResponse,
        stream: rquickjs::Persistent<Value<'static>>,
    },
}

/// `passthrough()` — hand the request back as unhandled (MSW parity).
/// Marker-header response instead of the Node package's Symbol sentinel;
/// the serve layer maps it to the unmatched path.
pub fn passthrough(ctx: Ctx<'_>) -> rquickjs::Result<Class<'_, HttpResponse>> {
    Class::instance(ctx, marked(crate::types::PASSTHROUGH_HEADER, None))
}

/// `bypass(input)` — identity. In Node the interceptor marks the request
/// to skip interception; a scripted handler runs outside any
/// interception, so there is nothing to bypass.
fn bypass(input: Opt<Value<'_>>) -> Option<Value<'_>> {
    input.0
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Class::<HttpResponse>::define(&ctx.globals())?;
    // Web-standard alias: MSW handlers routinely `return new Response(...)`.
    let ctor: Value<'_> = ctx.globals().get("HttpResponse")?;
    ctx.globals().set("Response", ctor)?;
    ctx.globals().set("passthrough", Func::from(passthrough))?;
    ctx.globals().set("bypass", Func::from(bypass))?;
    Ok(())
}

/// Convert a handler's resolved return value.
///
/// Accepted shapes, in precedence order:
/// - `HttpResponse` class instance — field move, no serialization; a
///   streaming body comes back as [`ConvertedResponse::Streaming`] for
///   the bridge to drain
/// - `undefined`/`null` — fall-through marker: the caller retries
///   matching with this mock excluded (MSW semantics)
/// - string — [`DynamicResponse::from_rendered_string`]
/// - any other value — QuickJS C JSON stringify, then
///   [`DynamicResponse::from_rendered_string`] structured-form parsing
///   (parses at most once; non-structured JSON passes through as-is)
pub fn value_to_dynamic_response<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> Result<ConvertedResponse, MockpitError> {
    if let Some(obj) = value.as_object()
        && let Some(resp) = Class::<HttpResponse>::from_object(obj)
    {
        return Ok(resp.borrow().to_converted());
    }
    let ready = if value.is_undefined() || value.is_null() {
        DynamicResponse::fallthrough()
    } else if let Some(s) = value.as_string() {
        let s = s
            .to_string()
            .map_err(|e| MockpitError::Script(format!("handler returned invalid string: {e}")))?;
        DynamicResponse::from_rendered_string(s)
    } else {
        let json = ctx
            .json_stringify(value)
            .map_err(|e| {
                MockpitError::Script(format!("handler return value not serializable: {e}"))
            })?
            .map(|s| s.to_string())
            .transpose()
            .map_err(|e| {
                MockpitError::Script(format!("handler return value not serializable: {e}"))
            })?
            .unwrap_or_else(|| "null".to_string());
        DynamicResponse::from_rendered_string(json)
    };
    Ok(ConvertedResponse::Ready(ready))
}
