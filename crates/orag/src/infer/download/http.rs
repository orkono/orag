//! The HTTP client of the model download: ureq over rustls (ring, Mozilla
//! roots), proxies from the environment, and reads that wait at most the
//! idle limit for the next bytes and wake often enough to notice a cancel,
//! enforced beneath TLS (see `watch`).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    ConnectProxyConnector, Connector, RustlsConnector, TcpConnector,
};

use crate::infer::download::watch::{ReadWatch, SocketWatch, Watch};

/// Longest wait for a connection (TCP and TLS) to be established.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest wait for the next bytes of a response before it is a network error.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How often a read waiting for bytes wakes to look at the cancel flag, so a
/// download stops well within a second of a cancel even on a stalled peer.
/// The wakes count toward the idle limit; they never end a read themselves.
pub const CANCEL_POLL: Duration = Duration::from_millis(250);
/// Redirects followed (Hugging Face sends one to its CDN).
const MAX_REDIRECTS: u32 = 5;

/// How the client reaches the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpSettings {
    /// Honour HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY.
    pub env_proxy: bool,
    /// Refuse plain `http://` URLs, redirects included.
    pub https_only: bool,
    pub idle_timeout: Duration,
    /// Longest time from starting a connection to finishing its TLS
    /// handshake (proxy CONNECT included).
    pub connect_timeout: Duration,
}

impl HttpSettings {
    /// What the service uses.
    pub fn production() -> HttpSettings {
        HttpSettings {
            env_proxy: true,
            https_only: true,
            idle_timeout: IDLE_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
        }
    }
}

/// A client with status codes as values, so every response can be checked.
/// Once `cancel` is set, every read waiting for bytes fails within
/// `CANCEL_POLL` with an I/O error, which the caller reports as the cancel.
pub fn agent(settings: &HttpSettings, cancel: Arc<AtomicBool>) -> ureq::Agent {
    agent_with(settings, cancel, None, None)
}

/// `agent` trusting only `root` instead of the Mozilla roots, and going
/// through the CONNECT proxy at `proxy` when given (never the environment's):
/// tests reach a local TLS server with a certificate they made. The service
/// never calls it.
#[cfg(test)]
pub fn agent_for_test(
    settings: &HttpSettings,
    cancel: Arc<AtomicBool>,
    root: ureq::tls::Certificate<'static>,
    proxy: Option<&str>,
) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::new_with_certs(&[root]))
        .build();
    let proxy = proxy.map(|uri| ureq::Proxy::new(uri).expect("a valid test proxy"));
    agent_with(settings, cancel, Some(tls), proxy)
}

fn agent_with(
    settings: &HttpSettings,
    cancel: Arc<AtomicBool>,
    tls: Option<ureq::tls::TlsConfig>,
    proxy: Option<ureq::Proxy>,
) -> ureq::Agent {
    let proxy = proxy.or_else(|| settings.env_proxy.then(ureq::Proxy::try_from_env).flatten());
    let mut config = ureq::Agent::config_builder();
    if let Some(tls) = tls {
        config = config.tls_config(tls);
    }
    let config = config
        .http_status_as_error(false)
        .https_only(settings.https_only)
        .proxy(proxy)
        .max_redirects(MAX_REDIRECTS)
        .timeout_connect(Some(settings.connect_timeout))
        .timeout_recv_response(Some(settings.idle_timeout))
        .user_agent(format!("orag/{}", env!("CARGO_PKG_VERSION")))
        .build();
    // ureq's `DefaultConnector` with the two watches added: one on the TCP
    // stream that rustls reads, one above TLS. Its other links only warn
    // about features this build does not use (SOCKS, native-tls).
    let watch = Watch::new(cancel, settings.idle_timeout);
    let connector =
        ().chain(ConnectProxyConnector::default())
            .chain(TcpConnector::default())
            .chain(SocketWatch(watch.clone()))
            .chain(RustlsConnector::default())
            .chain(ReadWatch(watch));
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}
