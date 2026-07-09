//! Native `FormData` and `File` classes plus the multipart/urlencoded
//! codecs behind `request.formData()` and `HttpResponse.formData()`.

// rquickjs method targets must take FromJs params owned and the
// macro-injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::cell::RefCell;
use std::rc::Rc;

use bytes::Bytes;
use rquickjs::function::Opt;
use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, TypedArray, Value, class::Trace};

#[derive(Clone)]
pub enum FormValue {
    Text(String),
    File {
        name: String,
        content_type: String,
        data: Bytes,
    },
}

/// Minimal `File`: enough for form-data round trips (`name`, `type`,
/// `size`, `text()`, `arrayBuffer()`).
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "File")]
pub struct File {
    #[qjs(skip_trace)]
    name: String,
    #[qjs(skip_trace)]
    content_type: String,
    #[qjs(skip_trace)]
    data: Bytes,
}

fn value_to_bytes(value: &Value<'_>) -> Option<Bytes> {
    if let Some(s) = value.as_string() {
        return s.to_string().ok().map(Bytes::from);
    }
    if let Some(ab) = value
        .as_object()
        .and_then(|o| rquickjs::ArrayBuffer::from_object(o.clone()))
    {
        return Some(Bytes::copy_from_slice(ab.as_bytes().unwrap_or_default()));
    }
    TypedArray::<u8>::from_value(value.clone())
        .ok()
        .map(|ta| Bytes::copy_from_slice(ta.as_bytes().unwrap_or_default()))
}

#[rquickjs::methods]
impl File {
    /// `new File(bits, name, { type })` — bits is an array of
    /// strings/ArrayBuffers/TypedArrays.
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        bits: Value<'js>,
        name: String,
        options: Opt<Object<'js>>,
    ) -> rquickjs::Result<Self> {
        let mut data = Vec::new();
        if let Some(arr) = bits.as_array() {
            for item in arr.iter::<Value<'js>>() {
                let item = item?;
                let Some(bytes) = value_to_bytes(&item) else {
                    return Err(Exception::throw_type(
                        &ctx,
                        "File bits must be strings, ArrayBuffers, or TypedArrays",
                    ));
                };
                data.extend_from_slice(&bytes);
            }
        } else if let Some(bytes) = value_to_bytes(&bits) {
            data.extend_from_slice(&bytes);
        } else if !bits.is_undefined() && !bits.is_null() {
            return Err(Exception::throw_type(
                &ctx,
                "File bits must be an array of strings, ArrayBuffers, or TypedArrays",
            ));
        }
        let content_type = match options.0 {
            Some(opts) => opts.get::<_, Option<String>>("type")?.unwrap_or_default(),
            None => String::new(),
        };
        Ok(Self {
            name,
            content_type,
            data: Bytes::from(data),
        })
    }

    #[qjs(get)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[qjs(get, rename = "type")]
    pub fn content_type(&self) -> String {
        self.content_type.clone()
    }

    #[qjs(get)]
    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }

    #[qjs(rename = "arrayBuffer")]
    pub fn array_buffer<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        Ok(rquickjs::ArrayBuffer::new_copy(ctx, &self.data)?
            .as_value()
            .clone())
    }
}

impl File {
    fn from_value(value: FormValue) -> Self {
        match value {
            FormValue::Text(text) => Self {
                name: String::new(),
                content_type: String::new(),
                data: Bytes::from(text),
            },
            FormValue::File {
                name,
                content_type,
                data,
            } => Self {
                name,
                content_type,
                data,
            },
        }
    }
}

/// Standard `FormData` over an ordered multimap.
#[derive(Trace, JsLifetime, Default)]
#[rquickjs::class(rename = "FormData")]
pub struct FormData {
    #[qjs(skip_trace)]
    entries: Rc<RefCell<Vec<(String, FormValue)>>>,
}

impl FormData {
    pub fn from_entries(entries: Vec<(String, FormValue)>) -> Self {
        Self {
            entries: Rc::new(RefCell::new(entries)),
        }
    }

    fn form_value<'js>(
        ctx: &Ctx<'js>,
        value: Value<'js>,
        filename: Option<String>,
    ) -> rquickjs::Result<FormValue> {
        if let Some(obj) = value.as_object()
            && let Some(file) = Class::<File>::from_object(obj)
        {
            let file = file.borrow();
            return Ok(FormValue::File {
                name: filename.unwrap_or_else(|| file.name.clone()),
                content_type: file.content_type.clone(),
                data: file.data.clone(),
            });
        }
        if let Some(s) = value.as_string() {
            let text = s.to_string()?;
            return Ok(match filename {
                Some(name) => FormValue::File {
                    name,
                    content_type: String::new(),
                    data: Bytes::from(text),
                },
                None => FormValue::Text(text),
            });
        }
        if let Some(bytes) = value_to_bytes(&value) {
            return Ok(FormValue::File {
                name: filename.unwrap_or_else(|| "blob".to_string()),
                content_type: String::new(),
                data: bytes,
            });
        }
        Err(Exception::throw_type(
            ctx,
            "FormData values must be strings, Files, ArrayBuffers, or TypedArrays",
        ))
    }

    fn value_to_js<'js>(ctx: &Ctx<'js>, value: &FormValue) -> rquickjs::Result<Value<'js>> {
        match value {
            FormValue::Text(text) => {
                Ok(rquickjs::String::from_str(ctx.clone(), text)?.into_value())
            }
            file @ FormValue::File { .. } => Ok(Class::instance(
                ctx.clone(),
                File::from_value(file.clone()),
            )?
            .as_value()
            .clone()),
        }
    }

    pub fn snapshot(&self) -> Vec<(String, FormValue)> {
        self.entries.borrow().clone()
    }
}

#[rquickjs::methods]
impl FormData {
    #[qjs(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
        filename: Opt<String>,
    ) -> rquickjs::Result<()> {
        let value = Self::form_value(&ctx, value, filename.0)?;
        self.entries.borrow_mut().push((name, value));
        Ok(())
    }

    pub fn set<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        value: Value<'js>,
        filename: Opt<String>,
    ) -> rquickjs::Result<()> {
        let value = Self::form_value(&ctx, value, filename.0)?;
        let mut entries = self.entries.borrow_mut();
        entries.retain(|(k, _)| *k != name);
        entries.push((name, value));
        Ok(())
    }

    pub fn get<'js>(&self, ctx: Ctx<'js>, name: String) -> rquickjs::Result<Value<'js>> {
        match self
            .entries
            .borrow()
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
        {
            Some(value) => Self::value_to_js(&ctx, &value),
            None => Ok(Value::new_null(ctx)),
        }
    }

    #[qjs(rename = "getAll")]
    pub fn get_all<'js>(&self, ctx: Ctx<'js>, name: String) -> rquickjs::Result<Vec<Value<'js>>> {
        self.entries
            .borrow()
            .iter()
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| Self::value_to_js(&ctx, v))
            .collect()
    }

    pub fn has(&self, name: String) -> bool {
        self.entries.borrow().iter().any(|(k, _)| *k == name)
    }

    pub fn delete(&self, name: String) {
        self.entries.borrow_mut().retain(|(k, _)| *k != name);
    }

    pub fn entries<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Vec<Vec<Value<'js>>>> {
        self.entries
            .borrow()
            .iter()
            .map(|(k, v)| {
                Ok(vec![
                    rquickjs::String::from_str(ctx.clone(), k)?.into_value(),
                    Self::value_to_js(&ctx, v)?,
                ])
            })
            .collect()
    }

    pub fn keys(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn values<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Vec<Value<'js>>> {
        self.entries
            .borrow()
            .iter()
            .map(|(_, v)| Self::value_to_js(&ctx, v))
            .collect()
    }

    #[qjs(rename = "forEach")]
    pub fn for_each(&self, callback: rquickjs::Function<'_>) -> rquickjs::Result<()> {
        let ctx = callback.ctx().clone();
        for (name, value) in self.entries.borrow().iter() {
            callback.call::<_, ()>((Self::value_to_js(&ctx, value)?, name.as_str()))?;
        }
        Ok(())
    }
}

/// Parse a request body into form entries based on its Content-Type.
pub fn parse_body(content_type: &str, body: &str) -> Result<Vec<(String, FormValue)>, String> {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match ct.as_str() {
        "application/x-www-form-urlencoded" => Ok(parse_urlencoded(body)),
        "multipart/form-data" => {
            let boundary = content_type
                .split(';')
                .find_map(|part| part.trim().strip_prefix("boundary="))
                .map(|b| b.trim_matches('"'))
                .ok_or("multipart/form-data without a boundary parameter")?;
            Ok(parse_multipart(body, boundary))
        }
        other => Err(format!(
            "cannot parse '{other}' as form data (expected multipart/form-data or application/x-www-form-urlencoded)"
        )),
    }
}

fn parse_urlencoded(body: &str) -> Vec<(String, FormValue)> {
    body.split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = decode_component(parts.next()?);
            let value = decode_component(parts.next().unwrap_or_default());
            Some((key, FormValue::Text(value)))
        })
        .collect()
}

fn decode_component(raw: &str) -> String {
    let plus_decoded = raw.replace('+', " ");
    urlencoding::decode(&plus_decoded)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or(plus_decoded)
}

fn parse_multipart(body: &str, boundary: &str) -> Vec<(String, FormValue)> {
    let delimiter = format!("--{boundary}");
    let mut entries = Vec::new();

    for raw_part in body.split(delimiter.as_str()).skip(1) {
        let part = raw_part.strip_prefix("\r\n").unwrap_or(raw_part);
        if part.starts_with("--") {
            break; // closing delimiter
        }
        let Some((raw_headers, raw_body)) = part.split_once("\r\n\r\n") else {
            continue;
        };
        // The part body runs up to the CRLF preceding the next delimiter.
        let content = raw_body.strip_suffix("\r\n").unwrap_or(raw_body);

        let mut name = None;
        let mut filename = None;
        let mut content_type = String::new();
        for header in raw_headers.split("\r\n") {
            let Some((header_name, header_value)) = header.split_once(':') else {
                continue;
            };
            match header_name.trim().to_ascii_lowercase().as_str() {
                "content-disposition" => {
                    for param in header_value.split(';') {
                        let param = param.trim();
                        if let Some(v) = param.strip_prefix("name=") {
                            name = Some(v.trim_matches('"').to_string());
                        } else if let Some(v) = param.strip_prefix("filename=") {
                            filename = Some(v.trim_matches('"').to_string());
                        }
                    }
                }
                "content-type" => content_type = header_value.trim().to_string(),
                _ => {}
            }
        }

        let Some(name) = name else { continue };
        let value = match filename {
            Some(filename) => FormValue::File {
                name: filename,
                content_type,
                data: Bytes::copy_from_slice(content.as_bytes()),
            },
            None => FormValue::Text(content.to_string()),
        };
        entries.push((name, value));
    }

    entries
}

/// Serialize form entries as multipart/form-data. Returns the body and
/// the boundary used.
pub fn serialize_multipart(entries: &[(String, FormValue)]) -> (Bytes, String) {
    let boundary = format!("mockpitformboundary{:032x}", rand::random::<u128>());
    let mut out = Vec::new();
    for (name, value) in entries {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match value {
            FormValue::Text(text) => {
                out.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                );
                out.extend_from_slice(text.as_bytes());
            }
            FormValue::File {
                name: filename,
                content_type,
                data,
            } => {
                out.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                    )
                    .as_bytes(),
                );
                let ct = if content_type.is_empty() {
                    "application/octet-stream"
                } else {
                    content_type
                };
                out.extend_from_slice(format!("Content-Type: {ct}\r\n\r\n").as_bytes());
                out.extend_from_slice(data);
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (Bytes::from(out), boundary)
}
