//! Native `URL` and `URLSearchParams` globals (WHATWG URL API subset).
//!
//! MSW handlers idiomatically do `new URL(request.url).searchParams` —
//! both classes are read/write value objects, but `url.searchParams`
//! returns a snapshot: mutating it does not write back into the URL.

// rquickjs method targets must take FromJs params owned and the
// macro-injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::cell::RefCell;

use rquickjs::function::Opt;
use rquickjs::{Class, Ctx, Exception, JsLifetime, Value, class::Trace};

use super::set_entries_iterator;

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "URLSearchParams")]
pub struct UrlSearchParams {
    #[qjs(skip_trace)]
    entries: RefCell<Vec<(String, String)>>,
}

fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    let query = query.strip_prefix('?').unwrap_or(query);
    let mut entries = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |s: &str| {
            urlencoding::decode(&s.replace('+', " "))
                .map_or_else(|_| s.to_string(), std::borrow::Cow::into_owned)
        };
        entries.push((decode(name), decode(value)));
    }
    entries
}

fn params_from_init<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<Vec<(String, String)>> {
    use rquickjs::convert::Coerced;

    if let Some(s) = value.as_string() {
        return Ok(parse_query_pairs(&s.to_string()?));
    }
    if let Some(obj) = value.as_object()
        && let Some(params) = Class::<UrlSearchParams>::from_object(obj)
    {
        return Ok(params.borrow().entries.borrow().clone());
    }
    if let Some(arr) = value.as_array() {
        let mut entries = Vec::with_capacity(arr.len());
        for item in arr.iter::<rquickjs::Array<'_>>() {
            let pair = item?;
            if pair.len() != 2 {
                return Err(Exception::throw_type(
                    ctx,
                    "URLSearchParams init array entries must be [name, value] pairs",
                ));
            }
            let name: Coerced<String> = pair.get(0)?;
            let value: Coerced<String> = pair.get(1)?;
            entries.push((name.0, value.0));
        }
        return Ok(entries);
    }
    if let Some(obj) = value.as_object() {
        let mut entries = Vec::new();
        for entry in obj.props::<String, Coerced<String>>() {
            let (name, value) = entry?;
            entries.push((name, value.0));
        }
        return Ok(entries);
    }
    Err(Exception::throw_type(
        ctx,
        "URLSearchParams init must be a string, URLSearchParams, array of pairs, or object",
    ))
}

#[rquickjs::methods]
impl UrlSearchParams {
    #[qjs(constructor)]
    pub fn new<'js>(ctx: Ctx<'js>, init: Opt<Value<'js>>) -> rquickjs::Result<Self> {
        let entries = match init.0 {
            None => Vec::new(),
            Some(v) if v.is_undefined() || v.is_null() => Vec::new(),
            Some(v) => params_from_init(&ctx, v)?,
        };
        Ok(Self {
            entries: RefCell::new(entries),
        })
    }

    #[qjs(get)]
    pub fn size(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn get(&self, name: String) -> Option<String> {
        self.entries
            .borrow()
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
    }

    #[qjs(rename = "getAll")]
    pub fn get_all(&self, name: String) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn has(&self, name: String, value: Opt<String>) -> bool {
        self.entries
            .borrow()
            .iter()
            .any(|(n, v)| *n == name && value.0.as_ref().is_none_or(|want| v == want))
    }

    pub fn set(&self, name: String, value: String) {
        let mut entries = self.entries.borrow_mut();
        entries.retain(|(n, _)| *n != name);
        entries.push((name, value));
    }

    pub fn append(&self, name: String, value: String) {
        self.entries.borrow_mut().push((name, value));
    }

    pub fn delete(&self, name: String, value: Opt<String>) {
        self.entries
            .borrow_mut()
            .retain(|(n, v)| *n != name || value.0.as_ref().is_some_and(|want| v != want));
    }

    #[qjs(rename = "forEach")]
    pub fn for_each(&self, callback: rquickjs::Function<'_>) -> rquickjs::Result<()> {
        for (k, v) in self.entries.borrow().clone() {
            callback.call::<_, ()>((v.as_str(), k.as_str()))?;
        }
        Ok(())
    }

    pub fn entries(&self) -> Vec<Vec<String>> {
        self.entries
            .borrow()
            .iter()
            .map(|(k, v)| vec![k.clone(), v.clone()])
            .collect()
    }

    pub fn keys(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn values(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn sort(&self) {
        self.entries.borrow_mut().sort_by(|(a, _), (b, _)| a.cmp(b));
    }

    #[qjs(rename = "toString")]
    pub fn to_string_js(&self) -> String {
        self.entries
            .borrow()
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "URL")]
pub struct Url {
    #[qjs(skip_trace)]
    inner: url::Url,
}

fn parse_url(ctx: &Ctx<'_>, input: &str, base: Option<&str>) -> rquickjs::Result<url::Url> {
    let result = match base {
        Some(base) => url::Url::parse(base).and_then(|b| b.join(input)),
        None => url::Url::parse(input),
    };
    result.map_err(|_| Exception::throw_type(ctx, &format!("Invalid URL: {input}")))
}

#[rquickjs::methods]
impl Url {
    #[qjs(constructor)]
    pub fn new(ctx: Ctx<'_>, input: String, base: Opt<String>) -> rquickjs::Result<Self> {
        Ok(Self {
            inner: parse_url(&ctx, &input, base.0.as_deref())?,
        })
    }

    #[qjs(static, rename = "canParse")]
    pub fn can_parse(input: String, base: Opt<String>) -> bool {
        match base.0 {
            Some(base) => url::Url::parse(&base).and_then(|b| b.join(&input)).is_ok(),
            None => url::Url::parse(&input).is_ok(),
        }
    }

    #[qjs(get)]
    pub fn href(&self) -> String {
        self.inner.to_string()
    }

    #[qjs(get)]
    pub fn origin(&self) -> String {
        self.inner.origin().ascii_serialization()
    }

    #[qjs(get)]
    pub fn protocol(&self) -> String {
        format!("{}:", self.inner.scheme())
    }

    #[qjs(get)]
    pub fn username(&self) -> String {
        self.inner.username().to_string()
    }

    #[qjs(get)]
    pub fn password(&self) -> String {
        self.inner.password().unwrap_or_default().to_string()
    }

    #[qjs(get)]
    pub fn host(&self) -> String {
        let host = self.inner.host_str().unwrap_or_default();
        match self.inner.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }
    }

    #[qjs(get)]
    pub fn hostname(&self) -> String {
        self.inner.host_str().unwrap_or_default().to_string()
    }

    #[qjs(get)]
    pub fn port(&self) -> String {
        self.inner.port().map(|p| p.to_string()).unwrap_or_default()
    }

    #[qjs(get)]
    pub fn pathname(&self) -> String {
        self.inner.path().to_string()
    }

    #[qjs(get)]
    pub fn search(&self) -> String {
        match self.inner.query() {
            Some(q) if !q.is_empty() => format!("?{q}"),
            _ => String::new(),
        }
    }

    #[qjs(get)]
    pub fn hash(&self) -> String {
        match self.inner.fragment() {
            Some(f) if !f.is_empty() => format!("#{f}"),
            _ => String::new(),
        }
    }

    #[qjs(get, rename = "searchParams")]
    pub fn search_params<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let entries = parse_query_pairs(self.inner.query().unwrap_or_default());
        Ok(Class::instance(
            ctx,
            UrlSearchParams {
                entries: RefCell::new(entries),
            },
        )?
        .as_value()
        .clone())
    }

    #[qjs(rename = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }

    #[qjs(rename = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_string()
    }
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Class::<Url>::define(&ctx.globals())?;
    Class::<UrlSearchParams>::define(&ctx.globals())?;
    set_entries_iterator::<UrlSearchParams>(ctx)?;
    Ok(())
}
