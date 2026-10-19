//! Cancel and the idle limit for every read of the download's connections,
//! enforced beneath TLS.
//!
//! rustls decodes whole records: one read of plaintext may take many socket
//! reads, and a peer that sends a record a few bytes at a time keeps each of
//! them short. A limit applied above rustls therefore never fires, and a
//! cancel is never seen, until the record is complete. So two layers share a
//! `Watch`:
//!
//! - `ReadWatch`, above TLS, starts a deadline when ureq waits for response
//!   bytes: the idle limit (or ureq's own, shorter, timeout). Progress is
//!   measured here, as plaintext delivered: bytes that do not complete a
//!   record do not move the deadline, so trickling cannot keep a read alive.
//!   A genuine 16 KiB record arrives far inside the limit on any link that can
//!   download a model at all.
//! - `SocketWatch`, on the TCP stream that rustls reads, waits in slices of at
//!   most `CANCEL_POLL`, checks the cancel flag between them, and ends the wait
//!   at that deadline. Without a `ReadWatch` wait in progress (the TLS
//!   handshake), it uses the timeout ureq passes, which ureq computes from the
//!   connect deadline.
//!
//! One agent serves one download run on one thread, and ureq reads one
//! connection at a time, so the agent's connections can share the deadline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ureq::unversioned::transport::time::Duration as WaitFor;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, NextTimeout, Transport};

use crate::infer::download::http::CANCEL_POLL;

/// The state both layers read: the cancel flag, the idle limit, and the
/// deadline of the read in progress.
#[derive(Debug)]
pub struct Watch {
    cancel: Arc<AtomicBool>,
    limit: Duration,
    wait: Mutex<Option<Wait>>,
}

#[derive(Debug, Clone, Copy)]
struct Wait {
    deadline: Instant,
    reason: ureq::Timeout,
}

impl Watch {
    pub fn new(cancel: Arc<AtomicBool>, limit: Duration) -> Arc<Watch> {
        Arc::new(Watch {
            cancel,
            limit,
            wait: Mutex::new(None),
        })
    }

    /// A wait starting now: ureq's timeout, capped at the idle limit.
    fn wait_from(&self, timeout: NextTimeout) -> Wait {
        let (after, reason) = if *timeout.after <= self.limit {
            (*timeout.after, timeout.reason)
        } else {
            (self.limit, ureq::Timeout::RecvBody)
        };
        Wait {
            deadline: Instant::now() + after,
            reason,
        }
    }

    fn capped(&self, timeout: NextTimeout) -> NextTimeout {
        if *timeout.after <= self.limit {
            return timeout;
        }
        NextTimeout {
            after: WaitFor::Exact(self.limit),
            reason: ureq::Timeout::RecvBody,
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<Wait>> {
        // A panic elsewhere cannot leave a `Copy` value half written.
        self.wait
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_cancel(&self) -> Result<(), ureq::Error> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(ureq::Error::Io(std::io::Error::other(
                "the download was cancelled",
            )));
        }
        Ok(())
    }
}

/// Wraps the transport ureq reads responses from (TLS, or TCP for `http://`).
#[derive(Debug)]
pub struct ReadWatch(pub Arc<Watch>);

/// Wraps the TCP transport that rustls reads.
#[derive(Debug)]
pub struct SocketWatch(pub Arc<Watch>);

impl<In: Transport> Connector<In> for ReadWatch {
    type Out = Watched<In, Above>;

    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| Watched::new(inner, &self.0)))
    }
}

impl<In: Transport> Connector<In> for SocketWatch {
    type Out = Watched<In, Below>;

    fn connect(
        &self,
        _: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| Watched::new(inner, &self.0)))
    }
}

/// The layer above TLS.
#[derive(Debug)]
pub struct Above;
/// The layer beneath TLS.
#[derive(Debug)]
pub struct Below;

#[derive(Debug)]
pub struct Watched<T, Layer> {
    inner: T,
    watch: Arc<Watch>,
    layer: std::marker::PhantomData<Layer>,
}

impl<T, Layer> Watched<T, Layer> {
    fn new(inner: T, watch: &Arc<Watch>) -> Watched<T, Layer> {
        Watched {
            inner,
            watch: watch.clone(),
            layer: std::marker::PhantomData,
        }
    }
}

impl<T: Transport> Transport for Watched<T, Above> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner
            .transmit_output(amount, self.watch.capped(timeout))
    }

    /// One wait for plaintext, under one deadline for all the socket reads
    /// it takes.
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.watch.check_cancel()?;
        *self.watch.slot() = Some(self.watch.wait_from(timeout));
        let result = self.inner.await_input(self.watch.capped(timeout));
        *self.watch.slot() = None;
        result
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

impl<T: Transport> Transport for Watched<T, Below> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner
            .transmit_output(amount, self.watch.capped(timeout))
    }

    /// Waits in slices of at most `CANCEL_POLL` until the deadline of the
    /// read in progress. A slice that times out consumes no bytes, so the
    /// socket is left as it was and waiting again is safe.
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let current = *self.watch.slot();
        let wait = current.unwrap_or_else(|| self.watch.wait_from(timeout));
        loop {
            self.watch.check_cancel()?;
            // A zero wait would read as "no timeout" to the socket.
            let left = wait.deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ureq::Error::Timeout(wait.reason));
            }
            let slice = NextTimeout {
                after: WaitFor::Exact(left.min(CANCEL_POLL)),
                reason: wait.reason,
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
