//! Handing JSON to the VM as native values.
//!
//! `rquickjs_serde::to_value` cannot be used for this. A transitive dependency
//! (rolldown) force-enables `serde_json/arbitrary_precision` workspace-wide,
//! and under that feature `serde_json::Value::Number`'s `Serialize` emits a
//! private one-key map that only serde_json's own deserializer intercepts.
//! Every other serializer takes it literally, so a number reaches a script as
//! `{"$serde_json::private::Number": "3"}`.
//!
//! Walking the value explicitly is also the faster path: it skips the serde
//! data model entirely rather than merely avoiding its number bug.
//!
//! The inverse direction stays on `rquickjs_serde::from_value` — reading a JS
//! value *into* Rust never touches the private-number token, and rquickjs-serde
//! already handles integral-float coercion, `undefined` properties, Proxies and
//! cycles.

use rquickjs::object::Property;
use rquickjs::{Ctx, Object, Value};
use serde_json::Value as JsonValue;

/// Build a native `rquickjs::Value` from a `serde_json::Value`.
pub fn json_to_js<'js>(ctx: &Ctx<'js>, value: &JsonValue) -> rquickjs::Result<Value<'js>> {
    match value {
        JsonValue::Null => Ok(Value::new_null(ctx.clone())),
        JsonValue::Bool(b) => Ok(Value::new_bool(ctx.clone(), *b)),
        JsonValue::Number(n) => {
            // `as_i64` / `as_f64` are the arbitrary-precision-safe accessors;
            // the tagged representation is only ever produced by the
            // `Serialize` impl, never by these.
            //
            // Integers take QuickJS's inline int tag rather than becoming a
            // heap double — which is the common case here, since counts and
            // totals arrive as `usize`. Going through `as_i64` also avoids a
            // float-to-int cast entirely.
            if let Some(i) = n.as_i64()
                && let Ok(i) = i32::try_from(i)
            {
                return Ok(Value::new_int(ctx.clone(), i));
            }
            Ok(Value::new_number(
                ctx.clone(),
                n.as_f64().unwrap_or(f64::NAN),
            ))
        }
        JsonValue::String(s) => Ok(rquickjs::String::from_str(ctx.clone(), s)?.into_value()),
        JsonValue::Array(items) => {
            let array = rquickjs::Array::new(ctx.clone())?;
            for (index, item) in items.iter().enumerate() {
                array.set(index, json_to_js(ctx, item)?)?;
            }
            Ok(array.into_value())
        }
        JsonValue::Object(map) => {
            let object = Object::new(ctx.clone())?;
            for (key, item) in map {
                define_own(&object, key.as_str(), json_to_js(ctx, item)?)?;
            }
            Ok(object.into_value())
        }
    }
}

/// Define `key` as an own data property, the way a JS object literal would.
///
/// Not `Object::set`: that routes through `[[Set]]`, so a `__proto__` key —
/// which a caller can put in an entity through `world.create` or the world's
/// HTTP API — would invoke the prototype setter instead of defining a field.
/// `Object::prop` lowers to `JS_DefineProperty`, which always creates an own
/// data property and never triggers a setter.
fn define_own<'js, V: rquickjs::IntoJs<'js>>(
    object: &Object<'js>,
    key: &str,
    value: V,
) -> rquickjs::Result<()> {
    object.prop(
        key,
        Property::from(value).writable().enumerable().configurable(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn eval_on(value: &JsonValue, script: &str) -> String {
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            let js = json_to_js(&ctx, value).unwrap();
            ctx.globals().set("subject", js).unwrap();
            ctx.eval::<String, _>(script).unwrap()
        })
    }

    /// The bug this module exists for: a number must arrive as a number.
    #[test]
    fn a_number_is_a_number_not_a_tagged_object() {
        let value = serde_json::json!({ "total": 3 });
        assert_eq!(eval_on(&value, "typeof subject.total"), "number");
        assert_eq!(eval_on(&value, "String(subject.total)"), "3");
    }

    #[test]
    fn a_float_keeps_its_value() {
        let value = serde_json::json!({ "ratio": 0.5 });
        assert_eq!(eval_on(&value, "typeof subject.ratio"), "number");
        assert_eq!(eval_on(&value, "String(subject.ratio)"), "0.5");
    }

    #[test]
    fn a_large_integer_stays_exact() {
        let value = serde_json::json!({ "n": 9_007_199_254_740_991i64 });
        assert_eq!(eval_on(&value, "String(subject.n)"), "9007199254740991");
    }

    #[test]
    fn primitives_and_containers_round_trip() {
        let value = serde_json::json!({
            "s": "text",
            "b": true,
            "nil": null,
            "list": [1, "two", false],
            "nested": { "deep": { "n": 7 } },
        });
        assert_eq!(eval_on(&value, "typeof subject.s"), "string");
        assert_eq!(eval_on(&value, "typeof subject.b"), "boolean");
        assert_eq!(eval_on(&value, "String(subject.nil)"), "null");
        assert_eq!(
            eval_on(&value, "String(Array.isArray(subject.list))"),
            "true"
        );
        assert_eq!(eval_on(&value, "String(subject.list.length)"), "3");
        assert_eq!(eval_on(&value, "String(subject.nested.deep.n)"), "7");
    }

    /// An entity can carry any field name a caller wrote, so a `__proto__`
    /// key must land as a field rather than retargeting the prototype.
    #[test]
    fn a_proto_key_becomes_a_field_not_a_prototype() {
        let value = serde_json::json!({ "__proto__": { "polluted": true }, "id": "1" });

        assert_eq!(
            eval_on(&value, "String(subject.polluted)"),
            "undefined",
            "the value must not have been installed as a prototype"
        );
        assert_eq!(
            eval_on(&value, "String(Object.hasOwn(subject, '__proto__'))"),
            "true",
            "it has to survive as an own data property"
        );
        assert_eq!(eval_on(&value, "String(({}).polluted)"), "undefined");
    }
}
