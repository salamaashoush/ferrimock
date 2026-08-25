//! Which upstream a request belongs to, and what its request line becomes there.

use super::config::RouteConfig;

/// Whether `path` sits under `prefix`, counting only whole path segments.
///
/// A prefix match on raw bytes would give `/apifoo` to the `/api` route. The
/// boundary check is what makes a prefix mean a mount point rather than a
/// string.
fn under_prefix(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/')
}

/// The first route claiming this request, or `None` when nothing does.
///
/// Routes are scanned in the order [`super::config::ProxyConfig::compile`]
/// left them, so the first hit is already the most specific one.
pub fn resolve<'routes>(
    routes: &'routes [RouteConfig],
    host: Option<&str>,
    path: &str,
) -> Option<&'routes RouteConfig> {
    routes.iter().find(|route| {
        under_prefix(path, &route.prefix) && route.host.matches(host.unwrap_or_default())
    })
}

/// Build the absolute URI a forwarded request carries.
///
/// Absolute rather than origin-form because the upstream client needs the
/// scheme to choose TLS and the authority to choose a pool entry; hyper
/// rewrites it back to origin-form on an HTTP/1.1 wire and into `:authority`
/// on an HTTP/2 one.
///
/// # Errors
/// Fails only when the joined path and query are not a legal URI, which means
/// the incoming request line was already malformed.
pub fn upstream_uri(
    route: &RouteConfig,
    path: &str,
    query: Option<&str>,
) -> crate::Result<http::Uri> {
    let forwarded_path = if route.strip_prefix && route.prefix != "/" {
        let stripped = path.strip_prefix(route.prefix.as_str()).unwrap_or(path);
        if stripped.is_empty() { "/" } else { stripped }
    } else {
        path
    };

    // `base_path` never ends in `/` and `forwarded_path` always begins with
    // one, so this is the only join and it cannot double the separator.
    let mut path_and_query = String::with_capacity(
        route.target.base_path.len() + forwarded_path.len() + query.map_or(0, |q| q.len() + 1),
    );
    path_and_query.push_str(&route.target.base_path);
    path_and_query.push_str(forwarded_path);
    if let Some(query) = query {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    http::Uri::builder()
        .scheme(route.target.scheme.clone())
        .authority(route.target.authority.clone())
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| crate::mp_err!("cannot build upstream URI for '{path}': {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::proxy::config::{HostMatch, ProxyConfig, Target};

    fn table(specs: &[&str]) -> Vec<RouteConfig> {
        let mut config = ProxyConfig {
            routes: specs
                .iter()
                .map(|s| RouteConfig::parse(s).unwrap())
                .collect(),
            ..ProxyConfig::default()
        };
        config.compile();
        config.routes
    }

    #[test]
    fn a_prefix_matches_only_whole_segments() {
        assert!(under_prefix("/api", "/api"));
        assert!(under_prefix("/api/users", "/api"));
        assert!(!under_prefix("/apifoo", "/api"));
        assert!(under_prefix("/anything", "/"));
    }

    #[test]
    fn the_longer_prefix_wins_regardless_of_declaration_order() {
        let routes = table(&["/=http://localhost:5173", "/api=http://localhost:8080"]);

        let api = resolve(&routes, None, "/api/users").unwrap();
        assert_eq!(api.target.authority.as_str(), "localhost:8080");

        let app = resolve(&routes, None, "/index.html").unwrap();
        assert_eq!(app.target.authority.as_str(), "localhost:5173");
    }

    #[test]
    fn a_named_host_outranks_a_wildcard_at_the_same_prefix() {
        let routes = table(&[
            "/=http://localhost:5173",
            "cdn.example.com/=http://localhost:9000",
        ]);

        let cdn = resolve(&routes, Some("cdn.example.com"), "/logo.png").unwrap();
        assert_eq!(cdn.target.authority.as_str(), "localhost:9000");

        let app = resolve(&routes, Some("localhost:3010"), "/logo.png").unwrap();
        assert_eq!(app.target.authority.as_str(), "localhost:5173");
    }

    #[test]
    fn nothing_matches_when_no_route_claims_the_path() {
        let routes = table(&["/api=http://localhost:8080"]);
        assert!(resolve(&routes, None, "/index.html").is_none());
    }

    #[test]
    fn the_upstream_uri_keeps_the_prefix_by_default() {
        let routes = table(&["/api=http://localhost:8080"]);
        let uri = upstream_uri(&routes[0], "/api/users", Some("page=2")).unwrap();
        assert_eq!(uri.to_string(), "http://localhost:8080/api/users?page=2");
    }

    #[test]
    fn stripping_a_prefix_leaves_a_root_path_rather_than_an_empty_one() {
        let route = RouteConfig::parse("/api=http://localhost:8080")
            .unwrap()
            .stripping_prefix();

        assert_eq!(
            upstream_uri(&route, "/api/users", None)
                .unwrap()
                .to_string(),
            "http://localhost:8080/users"
        );
        assert_eq!(
            upstream_uri(&route, "/api", None).unwrap().to_string(),
            "http://localhost:8080/"
        );
    }

    #[test]
    fn a_targets_base_path_is_prepended() {
        let route = RouteConfig::parse("/api=https://example.com/v2")
            .unwrap()
            .stripping_prefix();
        assert_eq!(
            upstream_uri(&route, "/api/folders", None)
                .unwrap()
                .to_string(),
            "https://example.com/v2/folders"
        );
    }

    #[test]
    fn a_route_matches_its_host_before_its_path() {
        let routes = vec![RouteConfig {
            host: HostMatch::parse("api.example.com"),
            ..RouteConfig::new("/", Target::parse("http://localhost:8080").unwrap())
        }];
        assert!(resolve(&routes, Some("api.example.com"), "/x").is_some());
        assert!(resolve(&routes, Some("other.example.com"), "/x").is_none());
    }
}
