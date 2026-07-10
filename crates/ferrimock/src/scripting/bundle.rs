//! rolldown bundler front-end for mock script files.
//!
//! Bundles a mock entry file (TypeScript ok; `node_modules` and shared
//! imports resolved + tree-shaken) into one ESM module, compiles it to
//! QuickJS bytecode once, and remaps error positions back to the
//! original sources. The only import surviving bundling is the
//! `ferrimock` native module, kept external so the bytecode re-links by
//! name against the loading runtime's `ModuleDef`.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use rolldown::{Bundler, BundlerOptions, InputItem, Platform, SourceMapType};
use rolldown_common::{Output, OutputFormat};
use rolldown_plugin::{
    HookResolveIdArgs, HookResolveIdOutput, HookResolveIdReturn, HookUsage, Plugin, PluginContext,
};
use rquickjs::module::WriteOptions;
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Module};

use crate::{FerrimockError, Result};

use super::bridge::caught_to_error;
use super::loader::FERRIMOCK_MODULE;

/// One bundled mock file compiled to QuickJS bytecode, plus the source
/// map to translate bundled positions back to the original sources.
pub struct CompiledBundle {
    pub module_name: String,
    pub bytecode: Arc<[u8]>,
    source_map: Option<sourcemap::SourceMap>,
}

#[derive(Debug)]
struct FerrimockRuntimePlugin;

impl Plugin for FerrimockRuntimePlugin {
    fn name(&self) -> Cow<'static, str> {
        "ferrimock-runtime".into()
    }

    // rolldown's Plugin trait requires async even for sync resolution.
    #[allow(clippy::unused_async_trait_impl)]
    async fn resolve_id(
        &self,
        _ctx: &PluginContext,
        args: &HookResolveIdArgs<'_>,
    ) -> HookResolveIdReturn {
        // The host module stays EXTERNAL: the emitted chunk keeps the
        // bare import and the bytecode re-links by name against the
        // loading runtime's ModuleDef.
        if args.specifier == FERRIMOCK_MODULE {
            return Ok(Some(HookResolveIdOutput {
                id: args.specifier.into(),
                external: Some(rolldown_common::ResolvedExternal::Bool(true)),
                ..Default::default()
            }));
        }
        Ok(None)
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::ResolveId
    }
}

/// Bundle + tree-shake + transpile the entry file into a single ESM
/// module. Returns the bundled code and the (hidden) source map JSON.
pub async fn bundle_source(entry: &Path, cwd: &Path) -> Result<(String, Option<String>)> {
    let options = BundlerOptions {
        input: Some(vec![InputItem {
            name: None,
            import: entry.to_string_lossy().into_owned(),
        }]),
        cwd: Some(cwd.to_path_buf()),
        // Neutral: no Node builtins are injected (QuickJS has none);
        // pure ESM/CJS node_modules still resolve and bundle.
        platform: Some(Platform::Neutral),
        format: Some(OutputFormat::Esm),
        // Hidden: emit the map but no `//# sourceMappingURL` trailer in
        // the code we feed to QuickJS.
        sourcemap: Some(SourceMapType::Hidden),
        ..Default::default()
    };

    let mut bundler = Bundler::with_plugins(options, vec![Arc::new(FerrimockRuntimePlugin)])
        .map_err(|e| FerrimockError::Script(format!("rolldown init: {e:?}")))?;
    // rolldown's generate future is large; box it so it doesn't bloat
    // the enclosing future.
    let out = Box::pin(bundler.generate())
        .await
        .map_err(|e| FerrimockError::Script(format!("{}: bundle: {e:?}", entry.display())))?;

    for asset in &out.assets {
        if let Output::Chunk(chunk) = asset
            && chunk.is_entry
        {
            let code = chunk.code.clone();
            return Ok(match &chunk.map {
                Some(m) => (code, Some(m.to_json_string())),
                None => (code, None),
            });
        }
    }
    Err(FerrimockError::Script(format!(
        "{}: rolldown produced no entry chunk",
        entry.display()
    )))
}

/// Bundle `entry` and compile it to bytecode, going through the disk
/// cache: an unchanged source tree skips both rolldown and the QuickJS
/// compile entirely.
pub async fn bundle_and_compile(entry: &Path, cwd: &Path) -> Result<CompiledBundle> {
    let module_name = entry.to_string_lossy().into_owned();

    let cache_key = super::bytecode_cache::entry_key(entry);
    if let Some(hit) = super::bytecode_cache::load(cache_key) {
        let source_map = hit
            .source_map_json
            .and_then(|j| sourcemap::SourceMap::from_slice(j.as_bytes()).ok());
        return Ok(CompiledBundle {
            module_name,
            bytecode: Arc::from(hit.bytecode.into_boxed_slice()),
            source_map,
        });
    }

    let (code, map_json) = Box::pin(bundle_source(entry, cwd)).await?;

    // QuickJS resolves the module graph EAGERLY at declare, and the
    // bundle keeps the `ferrimock` specifier external — so even this
    // throwaway compile runtime needs the native resolver/loader. The
    // written bytecode stores the dependency by NAME and re-links
    // against the loading runtime's own ModuleDef.
    let runtime = AsyncRuntime::new()
        .map_err(|e| FerrimockError::Script(format!("bytecode runtime: {e}")))?;
    runtime
        .set_loader(
            super::loader::native_resolver(),
            super::loader::native_loader(),
        )
        .await;
    let ctx = AsyncContext::full(&runtime)
        .await
        .map_err(|e| FerrimockError::Script(format!("bytecode context: {e}")))?;
    let name = module_name.clone();
    let bytecode: Vec<u8> = ctx
        .async_with(async |ctx| {
            let module = Module::declare(ctx.clone(), name.into_bytes(), code.into_bytes())
                .catch(&ctx)
                .map_err(|e| caught_to_error(&e))?;
            module
                .write(WriteOptions::default())
                .map_err(|e| FerrimockError::Script(format!("module write: {e}")))
        })
        .await?;

    let inputs = super::bytecode_cache::collect_inputs(entry, map_json.as_deref(), cwd);
    super::bytecode_cache::store(
        cache_key,
        &bytecode,
        &module_name,
        map_json.as_deref(),
        &inputs,
    );

    let source_map = map_json.and_then(|j| sourcemap::SourceMap::from_slice(j.as_bytes()).ok());
    Ok(CompiledBundle {
        module_name,
        bytecode: Arc::from(bytecode.into_boxed_slice()),
        source_map,
    })
}

impl CompiledBundle {
    /// Map a bundled-output `line:col` (1-based, as QuickJS reports)
    /// back to the original `.ts`/`.js` source location.
    #[must_use]
    pub fn remap(&self, line: u32, col: u32) -> Option<(String, u32, u32)> {
        let sm = self.source_map.as_ref()?;
        let token = sm.lookup_token(line.saturating_sub(1), col.saturating_sub(1))?;
        let src = token.get_source().unwrap_or("<unknown>").to_string();
        Some((src, token.get_src_line() + 1, token.get_src_col() + 1))
    }
}

/// Collect an `entry.ts -> original source` note for a load error when
/// the exception carries a bundled line number in its stack.
pub fn remap_error(err: FerrimockError, bundle: &CompiledBundle) -> FerrimockError {
    let FerrimockError::Script(message) = err else {
        return err;
    };
    // QuickJS stack frames look like "    at <fn> (<module>:<line>:<col>)"
    // (column optional). Rewrite frames pointing into the bundled module.
    let needle = format!("{}:", bundle.module_name);
    let remapped = message
        .lines()
        .map(|line| {
            let Some(idx) = line.find(&needle) else {
                return line.to_string();
            };
            let (prefix, rest) = (
                line.get(..idx).unwrap_or_default(),
                line.get(idx + needle.len()..).unwrap_or_default(),
            );
            let mut nums = rest
                .trim_end_matches(')')
                .splitn(2, ':')
                .filter_map(|n| n.parse::<u32>().ok());
            let (Some(l), c) = (nums.next(), nums.next().unwrap_or(1)) else {
                return line.to_string();
            };
            match bundle.remap(l, c) {
                Some((src, sl, sc)) => format!("{prefix}{src}:{sl}:{sc})"),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    FerrimockError::Script(remapped)
}
