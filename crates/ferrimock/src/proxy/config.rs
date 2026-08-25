//! What the proxy listens on, where it forwards, and what it is allowed to buffer.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// A parsed upstream target: everything needed to rebuild a request line
/// against it without re-parsing a URL per request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `http` or `https`.
    pub scheme: http::uri::Scheme,
    /// Host and port, already in the form a `Host` header wants.
    pub authority: http::uri::Authority,
    /// Base path every forwarded request is mounted under. Never has a
    /// trailing slash, so joining is always `base + path`.
    pub base_path: String,
}

impl Target {
    /// Parse `http://localhost:5173`, `https://api.example.com/v2`, or a bare
    /// `localhost:8080` (assumed http).
    ///
    /// # Errors
    /// Fails when the string is not a URL with a host.
    pub fn parse(raw: &str) -> crate::Result<Self> {
        let spelled = if raw.contains("://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        };

        let uri: http::Uri = spelled
            .parse()
            .map_err(|e| crate::mp_err!("invalid upstream URL '{raw}': {e}"))?;

        let scheme = uri.scheme().cloned().unwrap_or(http::uri::Scheme::HTTP);
        if scheme != http::uri::Scheme::HTTP && scheme != http::uri::Scheme::HTTPS {
            return Err(crate::mp_err!(
                "upstream URL '{raw}' has scheme '{scheme}'; only http and https can be proxied"
            ));
        }

        let authority = uri
            .authority()
            .cloned()
            .ok_or_else(|| crate::mp_err!("upstream URL '{raw}' names no host"))?;

        let base_path = uri.path().trim_end_matches('/').to_string();

        Ok(Self {
            scheme,
            authority,
            base_path,
        })
    }

    /// Whether reaching this target needs TLS.
    pub fn is_tls(&self) -> bool {
        self.scheme == http::uri::Scheme::HTTPS
    }

    /// The `ws://` or `wss://` origin for a WebSocket handshake against this target.
    pub fn ws_origin(&self) -> String {
        let scheme = if self.is_tls() { "wss" } else { "ws" };
        format!("{scheme}://{}", self.authority)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}{}", self.scheme, self.authority, self.base_path)
    }
}

/// Which requests a route claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMatch {
    /// Any `Host`.
    Any,
    /// Exactly this authority, compared without the port when the pattern
    /// carries none — a browser sends `Host: localhost:3000` and a rule
    /// written as `localhost` plainly means that host.
    Exact(String),
    /// `*.example.com`: the stored string is `.example.com`.
    Suffix(String),
}

impl std::fmt::Display for HostMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any => f.write_str("*"),
            Self::Exact(host) => f.write_str(host),
            Self::Suffix(suffix) => write!(f, "*{suffix}"),
        }
    }
}

impl HostMatch {
    /// Read `*`, `*.example.com` or `example.com` into a matcher.
    pub fn parse(raw: &str) -> Self {
        if raw == "*" {
            Self::Any
        } else if let Some(rest) = raw.strip_prefix("*.") {
            Self::Suffix(format!(".{}", rest.to_ascii_lowercase()))
        } else {
            Self::Exact(raw.to_ascii_lowercase())
        }
    }

    /// Whether a `Host` header value belongs to this matcher.
    pub fn matches(&self, host_header: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(want) => {
                let got = host_header.to_ascii_lowercase();
                if got == *want {
                    return true;
                }
                // A pattern with no port matches any port; one with a port
                // must agree exactly, which is what makes two dev servers on
                // one host distinguishable.
                !want.contains(':')
                    && got
                        .split_once(':')
                        .is_some_and(|(hostname, _)| hostname == want)
            }
            Self::Suffix(suffix) => {
                let got = host_header.to_ascii_lowercase();
                let hostname = got.split_once(':').map_or(got.as_str(), |(h, _)| h);
                hostname.ends_with(suffix.as_str())
            }
        }
    }
}

/// One forwarding rule.
#[derive(Debug, Clone)]
pub struct RouteConfig {
    /// Which `Host` this rule claims.
    pub host: HostMatch,
    /// Path prefix this rule claims. `/` claims everything.
    pub prefix: String,
    /// Where matching requests go.
    pub target: Target,
    /// Drop `prefix` from the path before forwarding, so `/api/users` reaches
    /// a backend mounted at `/users`.
    pub strip_prefix: bool,
    /// Forward the browser's `Host` instead of the target's.
    ///
    /// Off by default: a dev server comparing `Host` against `Origin` reads a
    /// mismatched pair as cross-origin and refuses the request. On, the
    /// `X-Forwarded-*` trio travels with it so the upstream can still
    /// reconstruct the URL the browser used.
    pub preserve_host: bool,
    /// Per-route override of the upstream response timeout.
    pub timeout: Option<Duration>,
}

impl RouteConfig {
    /// A route claiming every host at `prefix`.
    pub fn new(prefix: impl Into<String>, target: Target) -> Self {
        Self {
            host: HostMatch::Any,
            prefix: normalize_prefix(&prefix.into()),
            target,
            strip_prefix: false,
            preserve_host: false,
            timeout: None,
        }
    }

    /// Parse `[host]/prefix=upstream` shorthand, the form the CLI takes:
    /// `/api=http://localhost:8080`, `/=http://localhost:5173`, or
    /// `api.example.com/v2=https://staging.example.com`.
    ///
    /// # Errors
    /// Fails when there is no `=`, or when the upstream will not parse.
    pub fn parse(spec: &str) -> crate::Result<Self> {
        let (matcher, upstream) = spec.split_once('=').ok_or_else(|| {
            crate::mp_err!("route '{spec}' has no '=': write it as <path>=<upstream>")
        })?;

        let matcher = matcher.trim();
        let (host, prefix) = if matcher.is_empty() || matcher.starts_with('/') {
            (HostMatch::Any, matcher)
        } else if let Some(slash) = matcher.find('/') {
            let (host, path) = matcher.split_at(slash);
            (HostMatch::parse(host), path)
        } else {
            (HostMatch::parse(matcher), "/")
        };

        Ok(Self {
            host,
            prefix: normalize_prefix(prefix),
            target: Target::parse(upstream.trim())?,
            strip_prefix: false,
            preserve_host: false,
            timeout: None,
        })
    }

    /// Builder: drop the prefix before forwarding.
    #[must_use]
    pub fn stripping_prefix(mut self) -> Self {
        self.strip_prefix = true;
        self
    }

    /// Builder: forward the browser's `Host`.
    #[must_use]
    pub fn preserving_host(mut self) -> Self {
        self.preserve_host = true;
        self
    }
}

/// A prefix always starts with `/` and never ends with one, so
/// `prefix.len()` is both the comparison length and the strip length.
fn normalize_prefix(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// TLS termination on the listening side.
#[derive(Debug, Clone)]
pub enum TlsConfig {
    /// Generate a self-signed certificate at startup for these names.
    ///
    /// Nothing trusts it, which is the point: a dev proxy needs a secure
    /// context (service workers, `crypto.subtle`, `SameSite=None` cookies) far
    /// more often than it needs a chain that validates.
    SelfSigned {
        /// Subject alternative names to issue for.
        names: Vec<String>,
    },
    /// Use a certificate chain and key already on disk, in PEM.
    Files {
        /// PEM certificate chain, leaf first.
        cert: PathBuf,
        /// PEM private key (PKCS#8, PKCS#1 or SEC1).
        key: PathBuf,
    },
}

/// How the proxy talks to upstreams.
#[derive(Debug, Clone)]
pub struct UpstreamConfig {
    /// Give up on an upstream that has not sent response headers by then.
    /// Applies to headers only: a streaming body is allowed to run forever,
    /// which is the whole point of SSE.
    pub timeout: Option<Duration>,
    /// Idle connections kept per upstream host.
    pub pool_max_idle_per_host: usize,
    /// How long an idle pooled connection survives.
    pub pool_idle_timeout: Duration,
    /// Accept upstream certificates that do not validate.
    ///
    /// A dev backend behind a self-signed certificate is the normal case this
    /// exists for. It is a real hole, so it is never on by default.
    pub accept_invalid_certs: bool,
    /// Offer HTTP/2 to upstreams over ALPN. Off means every upstream
    /// connection is HTTP/1.1, which some older dev servers need.
    pub http2: bool,
    /// Give up on a TCP connect that has not completed.
    ///
    /// Separate from `timeout`, and much shorter: a dev server that is not
    /// running should say so at once rather than after a minute.
    pub connect_timeout: Option<Duration>,
    /// Bytes of HTTP/1.1 read buffer per upstream connection.
    pub http1_max_buf_size: usize,
    /// Largest HTTP/2 frame to send upstream.
    pub http2_max_frame_size: u32,
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(60)),
            pool_max_idle_per_host: 32,
            pool_idle_timeout: Duration::from_secs(90),
            accept_invalid_certs: false,
            http2: true,
            connect_timeout: Some(Duration::from_secs(10)),
            // hyper defaults to 8KB. A proxy reads whole responses off the
            // socket rather than sipping them, so a bigger buffer is fewer
            // syscalls per body at the cost of one buffer per connection.
            // Worth 6.5% on `proxy/forward/proxy/1MB` against 128KB
            // (1.088ms -> 1.017ms, p < 0.05) with the direct arm unmoved.
            http1_max_buf_size: 512 * 1024,
            // 16KB is the HTTP/2 minimum and hyper's default; raising it cuts
            // the frame count on a large body roughly proportionally.
            http2_max_frame_size: 64 * 1024,
        }
    }
}

/// Everything the proxy needs to run.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on.
    pub listen: SocketAddr,
    /// Forwarding rules, most specific first after [`ProxyConfig::compile`].
    pub routes: Vec<RouteConfig>,
    /// TLS termination, or plaintext when absent.
    pub tls: Option<TlsConfig>,
    /// Upstream client behaviour.
    pub upstream: UpstreamConfig,
    /// Cap on a request body buffered for mock matching or request patching.
    ///
    /// A body over the cap is streamed straight through and never offered to
    /// the matcher: a 2GB upload must not become a 2GB allocation because a
    /// mock somewhere matches on a JSON field.
    pub max_buffered_request: usize,
    /// Cap on a response body held for patching or recording. Over it, the
    /// response streams and is not recorded.
    pub max_buffered_response: usize,
    /// Serve mocks at all. Off makes this a plain reverse proxy, which is the
    /// configuration to measure overhead against.
    pub mocks_enabled: bool,
    /// Log one line per forwarded request.
    pub verbose: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 3010)),
            routes: Vec::new(),
            tls: None,
            upstream: UpstreamConfig::default(),
            max_buffered_request: 32 * 1024 * 1024,
            max_buffered_response: 32 * 1024 * 1024,
            mocks_enabled: true,
            verbose: false,
        }
    }
}

impl ProxyConfig {
    /// Sort routes so the first match is the most specific one.
    ///
    /// Resolution is a linear scan, which beats any map at the handful of
    /// routes a dev setup has, but only if the order already encodes
    /// specificity. A named host outranks `*`, and a longer prefix outranks a
    /// shorter one, so `/api` written after `/` still wins.
    pub fn compile(&mut self) {
        self.routes.sort_by(|a, b| {
            let host_rank = |h: &HostMatch| match h {
                HostMatch::Exact(_) => 0,
                HostMatch::Suffix(_) => 1,
                HostMatch::Any => 2,
            };
            host_rank(&a.host)
                .cmp(&host_rank(&b.host))
                .then_with(|| b.prefix.len().cmp(&a.prefix.len()))
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn target_defaults_to_http_and_drops_trailing_slash() {
        let target = Target::parse("localhost:5173").unwrap();
        assert_eq!(target.scheme, http::uri::Scheme::HTTP);
        assert_eq!(target.authority.as_str(), "localhost:5173");
        assert_eq!(target.base_path, "");

        let based = Target::parse("https://api.example.com/v2/").unwrap();
        assert!(based.is_tls());
        assert_eq!(based.base_path, "/v2");
    }

    #[test]
    fn target_refuses_a_scheme_it_cannot_speak() {
        let err = Target::parse("ftp://example.com").unwrap_err().to_string();
        assert!(err.contains("only http and https"), "{err}");
    }

    #[test]
    fn host_without_a_port_matches_any_port() {
        let matcher = HostMatch::parse("localhost");
        assert!(matcher.matches("localhost"));
        assert!(matcher.matches("localhost:3000"));
        assert!(!matcher.matches("notlocalhost"));
    }

    #[test]
    fn host_with_a_port_must_agree_exactly() {
        let matcher = HostMatch::parse("localhost:3000");
        assert!(matcher.matches("localhost:3000"));
        assert!(!matcher.matches("localhost:3001"));
        assert!(!matcher.matches("localhost"));
    }

    #[test]
    fn wildcard_host_matches_subdomains_only() {
        let matcher = HostMatch::parse("*.example.com");
        assert!(matcher.matches("api.example.com"));
        assert!(matcher.matches("api.example.com:443"));
        assert!(!matcher.matches("example.com"));
        assert!(!matcher.matches("api.example.org"));
    }

    #[test]
    fn route_spec_parses_path_only_and_host_qualified_forms() {
        let plain = RouteConfig::parse("/api=http://localhost:8080").unwrap();
        assert_eq!(plain.host, HostMatch::Any);
        assert_eq!(plain.prefix, "/api");
        assert_eq!(plain.target.authority.as_str(), "localhost:8080");

        let hosted = RouteConfig::parse("api.example.com/v2=https://staging.example.com").unwrap();
        assert_eq!(hosted.host, HostMatch::parse("api.example.com"));
        assert_eq!(hosted.prefix, "/v2");

        let bare_host = RouteConfig::parse("cdn.example.com=http://localhost:9000").unwrap();
        assert_eq!(bare_host.prefix, "/");
    }

    #[test]
    fn route_spec_without_an_equals_says_so() {
        let err = RouteConfig::parse("/api").unwrap_err().to_string();
        assert!(err.contains("no '='"), "{err}");
    }

    #[test]
    fn root_prefix_normalizes_to_a_single_slash() {
        assert_eq!(normalize_prefix("/"), "/");
        assert_eq!(normalize_prefix(""), "/");
        assert_eq!(normalize_prefix("/api/"), "/api");
        assert_eq!(normalize_prefix("api"), "/api");
    }

    #[test]
    fn a_host_matcher_prints_the_way_it_was_written() {
        assert_eq!(HostMatch::Any.to_string(), "*");
        assert_eq!(
            HostMatch::parse("cdn.example.com").to_string(),
            "cdn.example.com"
        );
        assert_eq!(
            HostMatch::parse("*.example.com").to_string(),
            "*.example.com"
        );
    }

    #[test]
    fn compile_puts_the_most_specific_route_first() {
        let mut config = ProxyConfig {
            routes: vec![
                RouteConfig::new("/", Target::parse("http://localhost:5173").unwrap()),
                RouteConfig::new("/api", Target::parse("http://localhost:8080").unwrap()),
                RouteConfig {
                    host: HostMatch::parse("cdn.example.com"),
                    ..RouteConfig::new("/", Target::parse("http://localhost:9000").unwrap())
                },
            ],
            ..ProxyConfig::default()
        };
        config.compile();

        assert_eq!(config.routes[0].host, HostMatch::parse("cdn.example.com"));
        assert_eq!(config.routes[1].prefix, "/api");
        assert_eq!(config.routes[2].prefix, "/");
    }
}
