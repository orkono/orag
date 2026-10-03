//! Loopback API protection (D-013). There is no authentication: the server
//! binds 127.0.0.1/::1 only, and these checks stop web pages in a local
//! browser from reaching it (DNS rebinding, cross-origin requests).

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;
use crate::server::errors::ApiError;

/// Blocks DNS-rebinding (non-loopback Host) and cross-origin browser requests.
pub async fn check_host_and_origin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !host_is_loopback(request.headers().get(header::HOST)) {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "Host must be a loopback address",
        )
        .into_response();
    }
    // A browser omits Origin on no-cors GETs and <script>/<img> loads but
    // still marks them cross-site.
    let cross_site = request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"cross-site"));
    if cross_site && request.headers().get(header::ORIGIN).is_none() {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden_origin",
            "cross-origin requests are not allowed",
        )
        .into_response();
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let allowed = origin
            .to_str()
            .is_ok_and(|o| origin_allowed(o, &state.allowed_origins));
        if !allowed {
            return ApiError::new(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "cross-origin requests are not allowed",
            )
            .into_response();
        }
    }
    next.run(request).await
}

/// A Host header is a name (or a bracketed IPv6 literal) followed by an
/// optional numeric port, nothing else; the name must be loopback.
pub fn host_is_loopback(value: Option<&HeaderValue>) -> bool {
    let Some(host) = value.and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (name, rest) = if let Some(inner) = host.strip_prefix('[') {
        match inner.split_once(']') {
            Some(parts) => parts,
            None => return false,
        }
    } else {
        let end = host.find(':').unwrap_or(host.len());
        (&host[..end], &host[end..])
    };
    // Digits only (no sign, no leading zeros) and a valid port number.
    let port_ok = rest.is_empty()
        || rest.strip_prefix(':').is_some_and(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && !(p.len() > 1 && p.starts_with('0'))
                && p.parse::<u16>().is_ok()
        });
    port_ok
        && (name.eq_ignore_ascii_case("localhost")
            || name
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.to_canonical().is_loopback()))
}

/// Origins are compared case-insensitively and without a trailing slash, the
/// form browsers send.
pub fn origin_allowed(origin: &str, allowed: &[String]) -> bool {
    let normal = |o: &str| o.trim_end_matches('/').to_ascii_lowercase();
    let origin = normal(origin);
    allowed.iter().any(|a| normal(a) == origin)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn loopback_hosts_are_accepted_with_or_without_port() {
        for host in [
            "localhost",
            "localhost:1",
            "127.0.0.1:7613",
            "127.0.0.2",
            "[::1]:7613",
            "[::1]",
        ] {
            assert!(
                host_is_loopback(Some(&HeaderValue::from_static(host))),
                "{host}"
            );
        }
    }

    #[test]
    fn other_hosts_are_rejected() {
        for host in [
            "evil.example",
            "localhost.evil.example",
            "10.0.0.1:7613",
            "[::2]:1",
            "",
        ] {
            assert!(
                !host_is_loopback(Some(&HeaderValue::from_static(host))),
                "{host}"
            );
        }
        assert!(!host_is_loopback(None));
    }

    #[test]
    fn spoofed_ports_and_userinfo_do_not_pass() {
        for host in [
            "127.0.0.1.evil.example",
            "evil.example:127",
            "localhost@evil.example",
        ] {
            assert!(
                !host_is_loopback(Some(&HeaderValue::from_static(host))),
                "{host}"
            );
        }
    }

    #[test]
    fn host_must_be_only_a_name_and_an_optional_numeric_port() {
        for host in [
            "localhost:1@evil.example",
            "localhost:evil.example",
            "[::1]evil.example",
            "[::1]:x",
            "localhost:+7613",
            "127.0.0.1:0007613",
            "localhost:",
            "127.0.0.1:99999",
        ] {
            assert!(
                !host_is_loopback(Some(&HeaderValue::from_static(host))),
                "{host}"
            );
        }
        assert!(host_is_loopback(Some(&HeaderValue::from_static(
            "[::ffff:127.0.0.1]:7613"
        ))));
    }

    #[test]
    fn allowed_origins_match_after_normalization() {
        let allowed = vec!["http://Tauri.localhost/".to_string()];
        assert!(origin_allowed("http://tauri.localhost", &allowed));
        assert!(origin_allowed("HTTP://TAURI.LOCALHOST/", &allowed));
        assert!(!origin_allowed("http://tauri.localhost.evil", &allowed));
    }
}
