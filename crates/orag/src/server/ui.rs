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
pub const APP_CSS: &str = include_str!("ui/app.css");
/// The page's script modules (`app.js` is the entry point), served at `/<name>`.
pub const SCRIPTS: [(&str, &str); 9] = [
    ("app.js", include_str!("ui/app.js")),
    ("api.js", include_str!("ui/api.js")),
    ("ask.js", include_str!("ui/ask.js")),
    ("collections.js", include_str!("ui/collections.js")),
    ("documents.js", include_str!("ui/documents.js")),
    ("dom.js", include_str!("ui/dom.js")),
    ("sources.js", include_str!("ui/sources.js")),
    ("sse.js", include_str!("ui/sse.js")),
    ("upload.js", include_str!("ui/upload.js")),
];
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
    let scripts = SCRIPTS.iter().map(|&(name, body)| (name, body, JAVASCRIPT));
    FILES
        .into_iter()
        .chain(scripts)
        .fold(Router::new(), |router, (name, body, content_type)| {
            router.route(
                &format!("/{name}"),
                get(move || async move { asset(body, content_type) }),
            )
        })
}

const JAVASCRIPT: &str = "text/javascript; charset=utf-8";

/// The page's other files: name (`""` is the page itself at `/`), body and
/// `Content-Type`. `routes` and `is_asset` both read this table and `SCRIPTS`.
const FILES: [(&str, &str, &str); 3] = [
    ("", INDEX_HTML, "text/html; charset=utf-8"),
    ("app.css", APP_CSS, "text/css; charset=utf-8"),
    ("favicon.svg", ICON_SVG, "image/svg+xml; charset=utf-8"),
];

/// Whether `path` is one of the page's static files (they carry no data).
pub fn is_asset(path: &str) -> bool {
    path.strip_prefix('/').is_some_and(|name| {
        FILES.iter().any(|&(file, _, _)| file == name)
            || SCRIPTS.iter().any(|&(script, _)| script == name)
    })
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
