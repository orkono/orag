//! CORS for the origins the starting program names (`serve --allow-origin`).
//! The desktop app's page lives on its own origin and calls this API
//! cross-origin; every other origin keeps getting the 403 from
//! `security::check_host_and_origin`, which runs before this middleware.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::server::AppState;
use crate::server::security::origin_allowed;

const ALLOW_METHODS: &str = "GET, POST, DELETE";
const ALLOW_HEADERS: &str = "accept, content-type";
/// Ten minutes: browsers cap the preflight cache lower anyway.
const MAX_AGE: &str = "600";
const REQUEST_PRIVATE_NETWORK: HeaderName =
    HeaderName::from_static("access-control-request-private-network");
const ALLOW_PRIVATE_NETWORK: HeaderName =
    HeaderName::from_static("access-control-allow-private-network");

/// Answers preflights and marks responses for an allowed `Origin`; anything
/// else (no Origin, or one not named) passes through untouched.
pub async fn cors(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let Some(origin) = allowed_origin(request.headers(), &state.allowed_origins) else {
        return next.run(request).await;
    };
    if request.method() == Method::OPTIONS {
        return preflight(origin, request.headers());
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("retry-after"),
    );
    headers.append(header::VARY, HeaderValue::from_static("origin"));
    response
}

/// The request's Origin as sent, when it is one of the named origins.
fn allowed_origin(headers: &HeaderMap, allowed: &[String]) -> Option<HeaderValue> {
    let origin = headers.get(header::ORIGIN)?;
    origin_allowed(origin.to_str().ok()?, allowed).then(|| origin.clone())
}

fn preflight(origin: HeaderValue, request_headers: &HeaderMap) -> Response {
    let mut response = Response::new(axum::body::Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOW_METHODS),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(ALLOW_HEADERS),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static(MAX_AGE),
    );
    headers.insert(header::VARY, HeaderValue::from_static("origin"));
    // Only named origins reach this point, so the grant is never wider.
    let wants_private = request_headers
        .get(REQUEST_PRIVATE_NETWORK)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"true"));
    if wants_private {
        headers.insert(ALLOW_PRIVATE_NETWORK, HeaderValue::from_static("true"));
    }
    response
}

/// Parses one `--allow-origin` value: `scheme://host[:port]`, nothing more
/// (one trailing `/` is allowed). Custom schemes such as `tauri://localhost`
/// are fine. The URL parser (WHATWG) judges the scheme, host and port; names
/// must also be DNS-style, which covers the opaque hosts of custom schemes.
/// Returns the canonical origin (see [`canonical_origin`]), the form a
/// browser sends in `Origin`.
pub fn parse_origin(value: &str) -> Result<String, String> {
    let authority = raw_authority(value)?;
    // The parser accepts an empty port (`host:`); an origin never has one.
    if authority.ends_with(':') {
        return Err("empty port after ':'".into());
    }
    let url = url::Url::parse(value)
        .map_err(|e| format!("not an origin ({e}), expected scheme://host[:port]"))?;
    let host = url
        .host()
        .ok_or("expected scheme://host[:port], with a host")?;
    if let url::Host::Domain(name) = host {
        check_dns_name(name)?;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("a user name or password is not part of an origin".into());
    }
    if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(NOT_ONLY_HOST_AND_PORT.into());
    }
    Ok(canonical_origin(&url))
}

const NOT_ONLY_HOST_AND_PORT: &str =
    "expected a host and port only, with no path, query or fragment";

/// The authority as typed: after `scheme://`, up to the first `/`, `?` or
/// `#`. What follows it must be empty or a single `/`. Checked on the raw
/// text because the parser normalises paths (`/a/..` becomes `/`).
fn raw_authority(value: &str) -> Result<&str, String> {
    let (_, rest) = value
        .split_once("://")
        .ok_or("expected scheme://host[:port]")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, remainder) = rest.split_at(end);
    if matches!(remainder, "" | "/") {
        Ok(authority)
    } else {
        Err(NOT_ONLY_HOST_AND_PORT.into())
    }
}

/// `scheme://host[:port]` from the parsed URL: the host as the parser
/// serialises it (lowercased, IDNA and percent-decoded for special schemes;
/// IPv6 in brackets), the port only when it is not the scheme's default.
fn canonical_origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

/// Labels of ASCII letters, digits and `-`, none empty and none starting or
/// ending with `-`; one trailing dot (a fully qualified name) is allowed.
fn check_dns_name(name: &str) -> Result<(), String> {
    let label_ok = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if name
        .strip_suffix('.')
        .unwrap_or(name)
        .split('.')
        .all(label_ok)
    {
        Ok(())
    } else {
        Err(format!("invalid host {name:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_origin;

    #[test]
    fn accepts_origins_and_drops_one_trailing_slash() {
        assert_eq!(
            parse_origin("tauri://localhost").unwrap(),
            "tauri://localhost"
        );
        assert_eq!(
            parse_origin("http://tauri.localhost/").unwrap(),
            "http://tauri.localhost"
        );
        for ok in [
            "http://[::1]:5173",
            "http://example.com.",
            "http://127.0.0.1:8080",
            "http://tauri.localhost",
            "tauri://localhost",
        ] {
            assert_eq!(parse_origin(ok).unwrap(), ok);
        }
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "not a url",
            "http://x/y",
            "http://",
            "://x",
            "http://a b",
            "http://u@x",
            "http://x?q=1",
            "1http://x",
            "http://:80",
            "http://localhost:bad",
            "http://localhost:99999",
            "http://localhost:",
            "http://[::1",
            "http://[::1]x",
            "http://[nope]:80",
            "http://[::1]:99999",
            "http://<bad>",
            "http://%zz",
            "http://a@b",
            "http://-x.com",
            "http://a..b",
            "HTTP://Bad Host",
            "http://a_b",
            "http://a-.com",
            "http://256.256.256.256",
            "tauri://%zz",
            "http://x/?q",
            "http://x/#f",
            "",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn stores_the_canonical_origin() {
        for (input, stored) in [
            ("HTTP://Example.COM:80/", "http://example.com"),
            ("http://%65xample.com", "http://example.com"),
            ("http://[::1]:8080", "http://[::1]:8080"),
            ("tauri://localhost", "tauri://localhost"),
        ] {
            assert_eq!(parse_origin(input).unwrap(), stored, "{input:?}");
        }
    }

    #[test]
    fn rejects_paths_the_parser_would_normalise() {
        for bad in ["http://x/a/..", "http://x/.", "http://x/?q", "http://x#f"] {
            assert!(parse_origin(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
