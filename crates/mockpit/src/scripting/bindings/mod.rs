//! Host bindings installed on every script VM: the MSW-compatible
//! surface (`http`, `graphql`, `HttpResponse`, `FormData`, `File`,
//! `ReadableStream`, `fake`, `delay`) plus `console`.

mod console;
mod delay;
mod fake;
pub mod form_data;
mod register;
pub mod request;
pub mod response;
pub mod sse;
pub mod streams;
mod url;
pub mod ws;

use rquickjs::atom::PredefinedAtom;
use rquickjs::function::{Func, This};
use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, Value, class::Trace};

/// `[Symbol.iterator]` for entry-list classes (Headers, FormData,
/// URLSearchParams): delegates to `this.entries()` and hands back the
/// array's own iterator, so `for...of` and spread work without a native
/// iterator protocol implementation.
// rquickjs Func targets take FromJs params owned and Ctx by value.
#[allow(clippy::needless_pass_by_value)]
fn entries_iterator<'js>(ctx: Ctx<'js>, this: This<Object<'js>>) -> rquickjs::Result<Value<'js>> {
    let entries_fn: rquickjs::Function<'js> = this.get("entries")?;
    let entries: Value<'js> = entries_fn.call((This(this.0.clone()),))?;
    let Some(arr) = entries.as_object() else {
        return Err(Exception::throw_type(
            &ctx,
            "entries() did not return an array",
        ));
    };
    let values_fn: rquickjs::Function<'js> = arr.get("values")?;
    values_fn.call((This(entries.clone()),))
}

fn set_entries_iterator<'js, C>(ctx: &Ctx<'js>) -> rquickjs::Result<()>
where
    C: rquickjs::class::JsClass<'js> + Trace<'js> + JsLifetime<'js>,
{
    if let Some(proto) = Class::<C>::prototype(ctx)? {
        proto.set(PredefinedAtom::SymbolIterator, Func::from(entries_iterator))?;
    }
    Ok(())
}

pub fn install_all(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Class::<request::RequestInfo>::define(&ctx.globals())?;
    Class::<request::Request>::define(&ctx.globals())?;
    Class::<request::Headers>::define(&ctx.globals())?;
    Class::<request::GraphQLRequestInfo>::define(&ctx.globals())?;
    Class::<form_data::FormData>::define(&ctx.globals())?;
    Class::<form_data::File>::define(&ctx.globals())?;
    Class::<streams::ReadableStream>::define(&ctx.globals())?;
    Class::<streams::StreamController>::define(&ctx.globals())?;
    set_entries_iterator::<request::Headers>(ctx)?;
    set_entries_iterator::<form_data::FormData>(ctx)?;
    response::install(ctx)?;
    register::install(ctx)?;
    sse::install(ctx)?;
    ws::install(ctx)?;
    url::install(ctx)?;
    fake::install(ctx)?;
    delay::install(ctx)?;
    console::install(ctx)?;
    Ok(())
}
