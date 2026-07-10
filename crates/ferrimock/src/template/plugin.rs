//! Plugin API for registering custom template functions
//!
//! Allows embedders to add template functions without depending on the
//! underlying template engine (tera). Functions receive arguments as
//! `HashMap<String, serde_json::Value>` and return `crate::Result<serde_json::Value>`.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A custom template function that can be registered by embedders.
///
/// Receives named arguments as a JSON value map and returns a JSON value.
/// Closures with the signature `Fn(&HashMap<String, Value>) -> crate::Result<Value>`
/// automatically implement this trait.
pub trait TemplateFunction: Send + Sync + 'static {
    fn call(&self, args: &HashMap<String, Value>) -> crate::Result<Value>;
}

impl<F> TemplateFunction for F
where
    F: Fn(&HashMap<String, Value>) -> crate::Result<Value> + Send + Sync + 'static,
{
    fn call(&self, args: &HashMap<String, Value>) -> crate::Result<Value> {
        self(args)
    }
}

struct RegisteredFunction {
    name: String,
    func: Arc<dyn TemplateFunction>,
}

static PLUGIN_FUNCTIONS: Mutex<Vec<RegisteredFunction>> = Mutex::new(Vec::new());

/// Register a custom template function by name.
///
/// Must be called before any templates are rendered (typically at application startup).
/// The function will be available in all mock templates as `{{ name(arg1=val1, ...) }}`.
///
/// # Example
///
/// ```rust,ignore
/// use ferrimock::template::register_template_function;
/// use serde_json::Value;
///
/// register_template_function("my_service_url", |args| {
///     let id = args.get("id")
///         .and_then(|v| v.as_str())
///         .unwrap_or("default");
///     Ok(Value::String(format!("https://my-service.com/{id}")))
/// });
/// ```
pub fn register_template_function(name: impl Into<String>, func: impl TemplateFunction) {
    if let Ok(mut functions) = PLUGIN_FUNCTIONS.lock() {
        functions.push(RegisteredFunction {
            name: name.into(),
            func: Arc::new(func),
        });
    }
}

/// Apply all registered plugin functions to a Tera instance.
/// Called internally when creating new thread-local Tera instances.
pub(super) fn apply_plugins(tera: &mut tera::Tera) {
    let Ok(functions) = PLUGIN_FUNCTIONS.lock() else {
        return;
    };
    for registered in functions.iter() {
        let func = Arc::clone(&registered.func);
        tera.register_function(
            &registered.name,
            move |args: &HashMap<String, Value>| -> tera::Result<Value> {
                func.call(args).map_err(tera::Error::msg)
            },
        );
    }
}
