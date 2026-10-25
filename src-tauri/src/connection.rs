//! The one open device connection, shared with the Tauri commands.
//!
//! The session is taken out of the mutex while an operation runs, so
//! `status()` never blocks on serial I/O. `generation` changes on connect and
//! disconnect so a slow, stale operation can't overwrite newer state.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use serde::Serialize;

use crate::commands::{DeviceInfo, SecurityStatus};
use crate::device::{DeviceError, Session};
use crate::ports::PortKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Disconnected,
    Connecting,
    Connected,
    Busy,
    Error,
}

/// What the frontend sees (event `connection-changed` and `connection_status`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    pub port: Option<String>,
    pub kind: Option<PortKind>,
    pub info: Option<DeviceInfo>,
    pub security: Option<SecurityStatus>,
    /// Set in the `error` state, or while `busy` (what is running).
    pub message: Option<String>,
}

#[derive(Default)]
struct Inner {
    state: Option<State>,
    port: Option<String>,
    kind: Option<PortKind>,
    info: Option<DeviceInfo>,
    security: Option<SecurityStatus>,
    message: Option<String>,
    session: Option<Session>,
    /// The background log/status worker has the session.
    live: bool,
    generation: u64,
    /// Set to abort the connect attempt in flight; a fresh flag is made per attempt.
    cancel: Arc<AtomicBool>,
}

impl Inner {
    fn state(&self) -> State {
        self.state.unwrap_or(State::Disconnected)
    }

    fn status(&self) -> Status {
        Status {
            state: self.state(),
            port: self.port.clone(),
            kind: self.kind,
            info: self.info.clone(),
            security: self.security.clone(),
            message: self.message.clone(),
        }
    }

    fn reset(&mut self, state: State, message: Option<String>) -> Option<Session> {
        log::info!(
            "connection: {:?} -> {:?}{}",
            self.state(),
            state,
            message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default()
        );
        if self.state() == State::Connecting {
            // Aborts a blocked read in the connect thread so it closes the port.
            self.cancel.store(true, Ordering::SeqCst);
        }
        self.generation += 1;
        self.state = Some(state);
        self.message = message;
        if state != State::Error {
            self.port = None;
            self.kind = None;
        }
        self.info = None;
        self.security = None;
        self.live = false;
        self.session.take()
    }
}

#[derive(Default)]
pub struct Connection(Mutex<Inner>, AtomicU32);

/// A session ready to use, plus the token to give back to `finish`.
pub struct Lease {
    pub session: Session,
    generation: u64,
}

impl Connection {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> Status {
        self.lock().status()
    }

    /// Moves to `connecting`. Returns the generation for `connected`/`failed`.
    pub fn begin_connect(&self, port: &str, kind: PortKind) -> Result<u64, String> {
        let mut inner = self.lock();
        if !matches!(inner.state(), State::Disconnected | State::Error) {
            return Err("Already connected. Disconnect first.".into());
        }
        let _ = inner.reset(State::Connecting, None);
        inner.port = Some(port.to_string());
        inner.kind = Some(kind);
        inner.cancel = Arc::new(AtomicBool::new(false));
        log::info!("connection: connecting to {port} ({kind:?}), attempt {}", inner.generation);
        Ok(inner.generation)
    }

    /// The abort flag of the connect attempt in flight (call right after `begin_connect`).
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.lock().cancel.clone()
    }

    /// Stores the opened session. Gives it back if the attempt was cancelled meanwhile.
    pub fn connected(&self, generation: u64, session: Session) -> Result<Status, Session> {
        let mut inner = self.lock();
        if inner.generation != generation {
            return Err(session);
        }
        inner.info = session.info.clone();
        inner.security = session.security.clone();
        inner.session = Some(session);
        inner.state = Some(State::Connected);
        inner.message = None;
        log::info!("connection: Connecting -> Connected");
        Ok(inner.status())
    }

    pub fn failed(&self, generation: u64, message: String) -> Option<Status> {
        let mut inner = self.lock();
        if inner.generation != generation {
            return None;
        }
        let _ = inner.reset(State::Error, Some(message));
        Some(inner.status())
    }

    /// Takes the session for one operation; the state becomes `busy`.
    pub fn lease(&self, what: &str) -> Result<(Lease, Status), String> {
        let mut inner = self.lock();
        match inner.state() {
            State::Connected => {}
            State::Busy => return Err("The device is busy with another operation.".into()),
            _ => return Err("Not connected to a device.".into()),
        }
        let session = inner.session.take().ok_or("Not connected to a device.")?;
        inner.state = Some(State::Busy);
        inner.message = Some(what.to_string());
        log::debug!("connection: Connected -> Busy ({what})");
        Ok((Lease { session, generation: inner.generation }, inner.status()))
    }

    /// Ends an operation. A transport error (or `keep == false`) drops the session.
    /// Returns the new status (None if superseded) and a session to drop off-thread.
    pub fn finish(&self, lease: Lease, outcome: &Result<(), DeviceError>, keep: bool) -> (Option<Status>, Option<Session>) {
        let mut inner = self.lock();
        if inner.generation != lease.generation {
            return (None, Some(lease.session));
        }
        match outcome {
            Err(DeviceError::Transport(m)) => {
                let message = format!("Connection lost: {m}");
                let _ = inner.reset(State::Error, Some(message));
                (Some(inner.status()), Some(lease.session))
            }
            _ if !keep => {
                let _ = inner.reset(State::Disconnected, None);
                (Some(inner.status()), Some(lease.session))
            }
            _ => {
                inner.security = lease.session.security.clone();
                inner.session = Some(lease.session);
                inner.state = Some(State::Connected);
                inner.message = None;
                (Some(inner.status()), None)
            }
        }
    }

    /// A user operation is about to lease: the background worker must stay off the
    /// link until `user_done`. Returns when the worker has handed the session back.
    pub fn user_wait_begin(&self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }

    pub fn user_wait_end(&self) {
        self.1.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn user_waiting(&self) -> bool {
        self.1.load(Ordering::SeqCst) > 0
    }

    pub fn live_active(&self) -> bool {
        self.lock().live
    }

    /// Whether a background worker still has a connection to use.
    pub fn is_connected(&self) -> bool {
        let inner = self.lock();
        matches!(inner.state(), State::Connected) && (inner.session.is_some() || inner.live)
    }

    /// Takes the session for the background worker without showing `busy`.
    /// None while a user operation runs or waits, or when not connected.
    pub fn lease_live(&self) -> Option<Lease> {
        let mut inner = self.lock();
        if self.user_waiting() || inner.state() != State::Connected || inner.live {
            return None;
        }
        let session = inner.session.take()?;
        inner.live = true;
        Some(Lease { session, generation: inner.generation })
    }

    /// Ends a background lease. State stays `connected`; a transport error drops the link.
    pub fn finish_live(&self, lease: Lease, outcome: &Result<(), DeviceError>) -> (Option<Status>, Option<Session>) {
        let mut inner = self.lock();
        if inner.generation != lease.generation {
            // Disconnected meanwhile: `live` belongs to whoever connected since.
            return (None, Some(lease.session));
        }
        inner.live = false;
        match outcome {
            Err(DeviceError::Transport(m)) => {
                let message = format!("Connection lost: {m}");
                let _ = inner.reset(State::Error, Some(message));
                (Some(inner.status()), Some(lease.session))
            }
            _ => {
                inner.security = lease.session.security.clone();
                inner.session = Some(lease.session);
                (None, None)
            }
        }
    }

    pub fn disconnect(&self) -> (Status, Option<Session>) {
        let mut inner = self.lock();
        let session = inner.reset(State::Disconnected, None);
        (inner.status(), session)
    }
}
