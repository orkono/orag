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
/// (a trailing `/` is dropped). Custom schemes such as `tauri://localhost`
/// are fine. Returns the origin in the form browsers send.
pub fn parse_origin(value: &str) -> Result<String, String> {
    let trimmed = value.strip_suffix('/').unwrap_or(value);
    let (scheme, host) = trimmed
        .split_once("://")
        .ok_or("expected scheme://host[:port], for example tauri://localhost")?;
    let scheme_ok = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return Err(format!("invalid scheme {scheme:?}"));
    }
    if host.is_empty() || host.contains(['/', '?', '#', '@', '\\']) {
        return Err("expected a host and port only, with no path, query or user".into());
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the host holds a space or control character".into());
    }
    check_host_and_port(host)?;
    Ok(trimmed.to_string())
}

/// `host[:port]`: the host is a DNS-style name, a dotted IPv4 address or a
/// bracketed IPv6 literal; a port after a colon is numeric, 0-65535.
fn check_host_and_port(authority: &str) -> Result<(), String> {
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (ip, rest) = inner.split_once(']').ok_or("unclosed [ in the host")?;
        ip.parse::<std::net::Ipv6Addr>()
            .map_err(|_| format!("invalid IPv6 address {ip:?}"))?;
        let port = match rest {
            "" => None,
            _ => Some(
                rest.strip_prefix(':')
                    .ok_or("junk after the ] of the host")?,
            ),
        };
        (None, port)
    } else {
        match authority.split_once(':') {
            Some((name, port)) => (Some(name), Some(port)),
            None => (Some(authority), None),
        }
    };
    if let Some(name) = host {
        check_dns_name(name)?;
    }
    match port {
        Some(p)
            if p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || p.parse::<u16>().is_err() =>
        {
            Err(format!("invalid port {p:?}, expected 0-65535"))
        }
        _ => Ok(()),
    }
}

/// Labels of ASCII letters, digits and `-`, none empty and none starting or
/// ending with `-`. A dotted IPv4 address is a name of digit labels.
fn check_dns_name(name: &str) -> Result<(), String> {
    let label_ok = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if name.split('.').all(label_ok) {
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
            "",
        ] {
            assert!(parse_origin(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
