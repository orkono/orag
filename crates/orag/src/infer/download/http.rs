//! The HTTP client of the model download: ureq over rustls (ring, Mozilla
//! roots), proxies from the environment, and a read that waits at most the
//! idle limit for the next bytes.

use std::time::Duration;

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

/// Longest wait for a connection (TCP and TLS) to be established.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest wait for the next bytes of a response before it is a network error.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
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
}

impl HttpSettings {
    /// What the service uses.
    pub fn production() -> HttpSettings {
        HttpSettings {
            env_proxy: true,
            https_only: true,
            idle_timeout: IDLE_TIMEOUT,
        }
    }
}

/// A client with status codes as values, so every response can be checked.
pub fn agent(settings: &HttpSettings) -> ureq::Agent {
    let proxy = settings.env_proxy.then(ureq::Proxy::try_from_env).flatten();
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .https_only(settings.https_only)
        .proxy(proxy)
        .max_redirects(MAX_REDIRECTS)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(settings.idle_timeout))
        .user_agent(format!("orag/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let connector = DefaultConnector::new().chain(IdleLimit(settings.idle_timeout));
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

/// ureq's body timeout is a budget for the whole body (hours for a 2.7 GB
/// file); this caps every single read instead, so a stalled peer is an error
/// after `IDLE_TIMEOUT` however long the transfer as a whole takes.
#[derive(Debug)]
struct IdleLimit(Duration);

impl Connector<Box<dyn Transport>> for IdleLimit {
    type Out = IdleTransport;

    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| IdleTransport {
            inner,
            limit: self.0,
        }))
    }
}

#[derive(Debug)]
struct IdleTransport {
    inner: Box<dyn Transport>,
    limit: Duration,
}

impl IdleTransport {
    fn capped(&self, timeout: NextTimeout) -> NextTimeout {
        if *timeout.after <= self.limit {
            return timeout;
        }
        NextTimeout {
            after: ureq::unversioned::transport::time::Duration::Exact(self.limit),
            reason: ureq::Timeout::RecvBody,
        }
    }
}

impl Transport for IdleTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let timeout = self.capped(timeout);
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let timeout = self.capped(timeout);
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
