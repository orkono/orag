//! Built-in web page (D-021): two actions, upload and ask, on top of the
//! public API. Served on its own loopback listener (`ui_bind`); the assets are
//! embedded, so the page works offline and loads nothing else (D-012).

use std::net::SocketAddr;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::server::AppState;

pub const INDEX_HTML: &str = include_str!("ui/index.html");
pub const APP_JS: &str = include_str!("ui/app.js");
pub const APP_CSS: &str = include_str!("ui/app.css");
/// The project icon (`assets/orag-icon.svg`), so the browser asks for no
/// `/favicon.ico`.
pub const ICON_SVG: &str = include_str!("../../../../assets/orag-icon.svg");

/// Only the page's own files: no inline script or style, no other host.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'";

/// The origins a browser sends for a page served at `addr`, the only ones the
/// API accepts besides requests without an `Origin`.
pub fn origins(addr: SocketAddr) -> Vec<String> {
    vec![
        format!("http://{addr}"),
        format!("http://localhost:{}", addr.port()),
    ]
}

/// The asset routes, added to the API routes on the UI listener only.
pub fn routes() -> Router<AppState> {
    ASSETS
        .iter()
        .fold(Router::new(), |router, &(path, body, content_type)| {
            router.route(path, get(move || async move { asset(body, content_type) }))
        })
}

/// Path, body and `Content-Type` of every file the page is made of.
const ASSETS: [(&str, &str, &str); 4] = [
    ("/", INDEX_HTML, "text/html; charset=utf-8"),
    ("/app.js", APP_JS, "text/javascript; charset=utf-8"),
    ("/app.css", APP_CSS, "text/css; charset=utf-8"),
    ("/favicon.svg", ICON_SVG, "image/svg+xml; charset=utf-8"),
];

/// Whether `path` is one of the page's static files (they carry no data).
pub fn is_asset(path: &str) -> bool {
    ASSETS.iter().any(|&(asset, _, _)| asset == path)
}

fn asset(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(CONTENT_SECURITY_POLICY),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
            // A new release must not run against the previous release's script.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}
