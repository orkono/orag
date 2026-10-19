//! The HTTP client of the model download: ureq over rustls (ring, Mozilla
//! roots), proxies from the environment, and a read that waits at most the
//! idle limit for the next bytes and wakes often enough to notice a cancel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::time::Duration as WaitFor;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

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
/// Once `cancel` is set, every read waiting for bytes fails within
/// `CANCEL_POLL` with an I/O error, which the caller reports as the cancel.
pub fn agent(settings: &HttpSettings, cancel: Arc<AtomicBool>) -> ureq::Agent {
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
    let limit = IdleLimit {
        limit: settings.idle_timeout,
        cancel,
    };
    let connector = DefaultConnector::new().chain(limit);
    ureq::Agent::with_parts(config, connector, DefaultResolver::default())
}

/// ureq's body timeout is a budget for the whole body (hours for a 2.7 GB
/// file); this caps every single read instead, so a stalled peer is an error
/// after `IDLE_TIMEOUT` however long the transfer as a whole takes.
#[derive(Debug)]
struct IdleLimit {
    limit: Duration,
    cancel: Arc<AtomicBool>,
}

impl Connector<Box<dyn Transport>> for IdleLimit {
    type Out = IdleTransport;

    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| IdleTransport {
            inner,
            limit: self.limit,
            cancel: self.cancel.clone(),
        }))
    }
}

#[derive(Debug)]
struct IdleTransport {
    inner: Box<dyn Transport>,
    limit: Duration,
    cancel: Arc<AtomicBool>,
}

impl IdleTransport {
    fn capped(&self, timeout: NextTimeout) -> NextTimeout {
        if *timeout.after <= self.limit {
            return timeout;
        }
        NextTimeout {
            after: WaitFor::Exact(self.limit),
            reason: ureq::Timeout::RecvBody,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
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

    /// Waits for bytes in slices of at most `CANCEL_POLL`, so a cancel is
    /// seen while the peer sends nothing; the slices add up to the limit one
    /// long wait had. A slice that times out consumes no bytes, so the TCP or
    /// TLS stream below is left as it was and waiting again is safe.
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let NextTimeout { after, reason } = self.capped(timeout);
        let started = Instant::now();
        loop {
            if self.cancelled() {
                return Err(ureq::Error::Io(std::io::Error::other(
                    "the download was cancelled",
                )));
            }
            // An expired deadline arrives as zero, which a socket would
            // read as "wait forever"; it is the timeout itself.
            let left = after.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(ureq::Error::Timeout(reason));
            }
            let slice = NextTimeout {
                after: WaitFor::Exact(left.min(CANCEL_POLL)),
                reason,
            };
            match self.inner.await_input(slice) {
                Err(ureq::Error::Timeout(_)) => continue,
                other => return other,
            }
        }
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
