//! Run the reverse proxy: mocks first, upstream for everything else.

use super::ui;
use anyhow::{Context, Result};
use ferrimock::engine::{MockMatcher, MockRegistry};
use ferrimock::proxy::{ProxyConfig, RouteConfig, Target, TlsConfig, UpstreamConfig};
use ferrimock::recorder::RecordingFormat;
use std::time::Duration;

/// Everything `ferrimock proxy` accepts.
pub struct ProxyOptions {
    pub upstream: Option<String>,
    pub routes: Vec<String>,
    pub port: u16,
    pub host: String,
    pub mocks: Option<String>,
    pub mock_file: Option<String>,
    pub watch: bool,
    pub strip_prefix: bool,
    pub preserve_host: bool,
    pub no_mocks: bool,
    pub tls: bool,
    pub tls_cert: Option<std::path::PathBuf>,
    pub tls_key: Option<std::path::PathBuf>,
    pub tls_names: Vec<String>,
    pub record: Option<String>,
    pub record_format: String,
    pub insecure: bool,
    pub no_http2: bool,
    pub timeout: u64,
    pub verbose: bool,
}

/// Start the proxy and run until interrupted.
pub async fn run(options: ProxyOptions) -> Result<()> {
    let routes = build_routes(&options)?;
    if routes.is_empty() {
        anyhow::bail!(
            "no upstream given. Pass one as a positional argument \
             (ferrimock proxy http://localhost:5173) or with --route </prefix=upstream>"
        );
    }

    let matcher = if options.no_mocks {
        None
    } else {
        Some(load_mocks(&options).await?)
    };

    let tls = build_tls(&options)?;

    let config = ProxyConfig {
        listen: format!("{}:{}", options.host, options.port)
            .parse()
            .with_context(|| format!("cannot bind {}:{}", options.host, options.port))?,
        routes,
        tls,
        upstream: UpstreamConfig {
            timeout: (options.timeout > 0).then(|| Duration::from_secs(options.timeout)),
            accept_invalid_certs: options.insecure,
            http2: !options.no_http2,
            ..UpstreamConfig::default()
        },
        mocks_enabled: !options.no_mocks,
        verbose: options.verbose,
        ..ProxyConfig::default()
    };

    let proxy = ferrimock::proxy::start(config, matcher)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if let Some(directory) = &options.record {
        let format =
            RecordingFormat::parse(&options.record_format).map_err(|e| anyhow::anyhow!("{e}"))?;
        let session = proxy
            .state()
            .start_recording(directory, None, format)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        crate::say!(
            "{}",
            ui::info(&format!(
                "recording forwarded traffic to {}",
                ui::path(&format!("{directory}/{session}"))
            ))
        );
    }

    report(&proxy, &options);

    // A failed signal registration means no Ctrl-C handler, which is a
    // reason to stop waiting rather than a reason to fail.
    drop(tokio::signal::ctrl_c().await);
    crate::say!();
    crate::say!("{}", ui::info("shutting down"));

    if options.record.is_some() {
        match proxy.state().stop_recording().await {
            Ok(Some(path)) => crate::say!(
                "{}",
                ui::success(&format!(
                    "recording written to {}",
                    ui::path(&path.display().to_string())
                ))
            ),
            Ok(None) => {}
            Err(error) => crate::say!(
                "{}",
                ui::warning(&format!("recording not written: {error}"))
            ),
        }
    }

    proxy.wait().await;
    Ok(())
}

/// Turn the positional upstream and every `--route` into a route table.
fn build_routes(options: &ProxyOptions) -> Result<Vec<RouteConfig>> {
    let mut routes = Vec::with_capacity(options.routes.len() + 1);

    for spec in &options.routes {
        let mut route = RouteConfig::parse(spec).map_err(|e| anyhow::anyhow!("{e}"))?;
        route.strip_prefix = options.strip_prefix;
        route.preserve_host = options.preserve_host;
        routes.push(route);
    }

    // The positional form is the common case spelled short: one dev server,
    // everything goes to it. It is added last so an explicit `--route /` wins.
    if let Some(upstream) = &options.upstream {
        let target = Target::parse(upstream).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut route = RouteConfig::new("/", target);
        route.preserve_host = options.preserve_host;
        routes.push(route);
    }

    Ok(routes)
}

async fn load_mocks(options: &ProxyOptions) -> Result<MockMatcher> {
    let registry = MockRegistry::new();
    let results = ferrimock::services::serve::load_mocks(
        &registry,
        options.mocks.as_deref(),
        options.mock_file.as_deref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let total: usize = results.iter().map(|result| result.count).sum();
    for result in &results {
        crate::say!(
            "{}",
            ui::info(&format!(
                "loaded {} mock(s) from {}",
                ui::number(result.count),
                ui::path(&result.source)
            ))
        );
    }

    if options.watch {
        let directory = options
            .mocks
            .clone()
            .unwrap_or_else(|| "mocks/collections".to_string());
        crate::say!(
            "{}",
            ui::info(&format!("watching {}", ui::path(&directory)))
        );
    }

    if total == 0 {
        crate::say!(
            "{}",
            ui::warning("no mocks loaded: every request will be forwarded")
        );
    }

    let registry = std::sync::Arc::new(registry);
    let matcher = MockMatcher::new((*registry).clone());
    // Nothing downstream of a mock miss here is a failure -- it forwards --
    // so tracking every miss would report normal traffic as unmatched.
    matcher.set_track_unmatched(false);
    Ok(matcher)
}

fn build_tls(options: &ProxyOptions) -> Result<Option<TlsConfig>> {
    match (&options.tls_cert, &options.tls_key) {
        (Some(cert), Some(key)) => Ok(Some(TlsConfig::Files {
            cert: cert.clone(),
            key: key.clone(),
        })),
        (Some(_), None) | (None, Some(_)) => {
            anyhow::bail!("--tls-cert and --tls-key have to be given together")
        }
        (None, None) if options.tls => Ok(Some(TlsConfig::SelfSigned {
            names: if options.tls_names.is_empty() {
                vec!["localhost".to_string(), "127.0.0.1".to_string()]
            } else {
                options.tls_names.clone()
            },
        })),
        (None, None) => Ok(None),
    }
}

fn report(proxy: &ferrimock::proxy::ProxyHandle, options: &ProxyOptions) {
    crate::say!();
    crate::say!("{}", ui::header("ferrimock proxy"));
    crate::say!("{}", ui::kv("listening", &ui::code(&proxy.url())));

    // Printed in resolution order, and a host-scoped route says so: two rules
    // can share a prefix and differ only by Host, and a listing that hides
    // that is unreadable exactly when it matters most.
    let routes = &proxy.state().config.routes;
    let host_scoped = routes
        .iter()
        .any(|route| route.host != ferrimock::proxy::HostMatch::Any);

    for route in routes {
        let matcher = if host_scoped {
            format!("{}{}", route.host, route.prefix)
        } else {
            route.prefix.clone()
        };
        let mut line = format!(
            "{} -> {}",
            ui::code(&matcher),
            ui::path(&route.target.to_string())
        );
        if route.strip_prefix {
            line.push_str(" (prefix stripped)");
        }
        if route.preserve_host {
            line.push_str(" (host preserved)");
        }
        crate::say!("{}", ui::list_item(&line));
    }

    if options.no_mocks {
        crate::say!("{}", ui::kv("mocks", "disabled (plain gateway)"));
    }
    if options.tls || options.tls_cert.is_some() {
        crate::say!("{}", ui::kv("tls", "on (http/1.1 and h2 over ALPN)"));
    }

    crate::say!();
    crate::say!(
        "{}",
        ui::info("point the browser here instead of the dev server")
    );
    crate::say!("{}", ui::sub_item("Ctrl-C to stop"));
}
