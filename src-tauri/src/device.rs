//! A connected device session: one transport plus the request/response
//! mechanics shared by every command. Protocol flows live in `commands.rs`.

use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::commands::{DeviceInfo, SecurityStatus};
use crate::framing::{Codec, Decoded};
use crate::ports::PortKind;
use crate::transport::Transport;

const READ_CHUNK: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceError {
    /// No complete reply before the deadline.
    Timeout,
    /// The reply didn't have the expected shape.
    Malformed(String),
    /// The device answered, but said no.
    Rejected(String),
    /// We refused to send something (bad input).
    Invalid(String),
    /// The port failed; the session is unusable.
    Transport(String),
}

impl DeviceError {
    /// Worth repeating the exchange: the link is fine, the reply wasn't.
    pub fn is_retryable(&self) -> bool {
        matches!(self, DeviceError::Timeout | DeviceError::Malformed(_))
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Timeout => write!(f, "the device didn't answer in time"),
            DeviceError::Malformed(m) => write!(f, "unexpected reply from the device: {m}"),
            DeviceError::Rejected(m) | DeviceError::Invalid(m) => write!(f, "{m}"),
            DeviceError::Transport(m) => write!(f, "connection lost: {m}"),
        }
    }
}

fn transport_error(e: std::io::Error) -> DeviceError {
    DeviceError::Transport(e.to_string())
}

/// Per-transport timing, from docs/spec/01-transport.md §2.4 and §3 and
/// 03-commands §2-6. Serializable so callers can override any of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeouts {
    /// Overall limit for one command and its reply.
    pub command_ms: u64,
    /// "Read until idle" finishes after this much silence.
    pub idle_ms: u64,
    /// Full-configuration read: per attempt, how many attempts, and the pause between them.
    pub config_read_ms: u64,
    pub config_read_attempts: u32,
    pub config_read_retry_delay_ms: u64,
    /// Committing the configuration to flash.
    // TODO(spec): the FMB640 family uses 20 s on USB (03-commands §4.3), but telling
    // that family apart needs the hardware table from unreleased 04-parameters.
    // 30 s is the longer of the two, so it's safe meanwhile.
    pub save_ms: u64,
}

impl Timeouts {
    pub fn for_kind(kind: PortKind) -> Self {
        let bluetooth = kind == PortKind::Bluetooth;
        Self {
            command_ms: 30_000,
            idle_ms: if bluetooth { 1_000 } else { 200 },
            config_read_ms: if bluetooth { 90_000 } else { 30_000 },
            config_read_attempts: 10,
            config_read_retry_delay_ms: 1_000,
            save_ms: 30_000,
        }
    }

    pub fn command(&self) -> Duration {
        Duration::from_millis(self.command_ms)
    }
}

pub struct Session {
    transport: Box<dyn Transport>,
    kind: PortKind,
    pub timeouts: Timeouts,
    /// Bytes received but not yet claimed by a reply.
    rx: Vec<u8>,
    /// Sent best-effort when the session is dropped, so the device leaves
    /// configuration mode whenever the port closes (01-transport §1.3).
    pub farewell: Option<Vec<u8>>,
    pub info: Option<DeviceInfo>,
    pub security: Option<SecurityStatus>,
    /// The device-side debug log is on (`.log:1` sent). `send` turns it off first so
    /// log text never mixes into a reply.
    pub log_streaming: bool,
    /// Human name of the link for error messages, e.g. "Bluetooth serial port".
    pub link: String,
}

/// Short, secret-free name of a command for logs and errors: ":cfg_setparam", ":sec_*".
pub fn command_label(bytes: &[u8]) -> String {
    if crate::diag::classify_command(bytes) == crate::diag::Ctx::Secret {
        return ":sec_*".into();
    }
    let text = String::from_utf8_lossy(bytes);
    let t = text.trim_end_matches(['\r', '\n']);
    let end = t.char_indices().skip(1).find(|(_, c)| *c == ':').map(|(i, _)| i).unwrap_or(t.len());
    t[..end].chars().take(24).collect()
}

fn default_link(kind: PortKind) -> &'static str {
    match kind {
        PortKind::Usb => "USB serial port",
        PortKind::Bluetooth => "Bluetooth serial port",
        PortKind::Other => "serial port",
    }
}

impl Session {
    pub fn new(transport: Box<dyn Transport>, kind: PortKind, timeouts: Timeouts) -> Self {
        Self {
            transport,
            kind,
            timeouts,
            rx: Vec::new(),
            farewell: None,
            info: None,
            security: None,
            log_streaming: false,
            link: default_link(kind).into(),
        }
    }

    /// "No reply to <step> over <link> after N s", naming the connect step that failed.
    pub fn step_failed(&self, step: &str, error: &DeviceError, started: Instant, limit: Duration) -> DeviceError {
        let elapsed = started.elapsed();
        log::warn!("session: step '{step}' failed after {} ms: {error}", elapsed.as_millis());
        match error {
            DeviceError::Timeout => {
                let hint = if self.kind == PortKind::Bluetooth {
                    ". Check the tracker is paired, in range and not connected to another program."
                } else {
                    ""
                };
                DeviceError::Rejected(format!("No reply to {step} over {} after {} s{hint}", self.link, limit.as_secs().max(1)))
            }
            DeviceError::Transport(m) if m == "cancelled" => DeviceError::Transport(m.clone()),
            DeviceError::Transport(m) => DeviceError::Rejected(format!("Lost the {} during {step}: {m}", self.link)),
            DeviceError::Malformed(m) => DeviceError::Rejected(format!("Unexpected reply to {step} over {}: {m}", self.link)),
            other => other.clone(),
        }
    }

    /// The error as shown to the user, with a hint where the link type matters.
    pub fn explain(&self, error: &DeviceError) -> String {
        match (self.kind, error) {
            (PortKind::Bluetooth, DeviceError::Timeout) => format!(
                "{error}. Check the tracker is paired, in range and not connected to another program."
            ),
            _ => error.to_string(),
        }
    }

    /// Discards pending input, then writes: stale bytes never leak into the next
    /// reply (01-transport §2B).
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        if self.log_streaming {
            // TODO(spec): 01/03 don't say how log text is told apart from replies, so the
            // log is always stopped (and drained) before any command.
            self.stop_log()?;
        }
        self.rx.clear();
        self.transport.clear_input().map_err(transport_error)?;
        log::debug!("session: send {} ({} bytes)", command_label(bytes), bytes.len());
        self.transport.write_all(bytes).map_err(transport_error)
    }

    /// Turns the debug log stream on (03-commands §8: `.log:1`, no reply awaited).
    pub fn start_log(&mut self) -> Result<(), DeviceError> {
        self.rx.clear();
        self.transport.clear_input().map_err(transport_error)?;
        self.transport.write_all(b".log:1\r").map_err(transport_error)?;
        self.log_streaming = true;
        Ok(())
    }

    /// Turns the stream off and returns the log text that was still in flight
    /// (read until idle, 01-transport §3; silence is fine).
    pub fn stop_log(&mut self) -> Result<Vec<u8>, DeviceError> {
        self.log_streaming = false;
        log::debug!("session: stopping device log stream");
        self.transport.write_all(b".log:0\r").map_err(transport_error)?;
        let wait = Duration::from_millis(self.timeouts.idle_ms * 2);
        match self.read_until_idle(wait) {
            Ok(data) => Ok(data),
            Err(DeviceError::Timeout) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Whatever arrives within `wait` (log streaming); empty if nothing did.
    pub fn read_available(&mut self, wait: Duration) -> Result<Vec<u8>, DeviceError> {
        let mut data = std::mem::take(&mut self.rx);
        if data.is_empty() {
            let mut chunk = [0u8; READ_CHUNK];
            let n = self.transport.read(&mut chunk, Instant::now() + wait).map_err(transport_error)?;
            data.extend_from_slice(&chunk[..n]);
        }
        Ok(data)
    }

    /// Waits for one reply. Bytes after it stay buffered for a following
    /// `receive`, for commands that answer with more than one line.
    pub fn receive<C: Codec>(&mut self, codec: &mut C, timeout: Duration) -> Result<C::Response, DeviceError> {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            match codec.decode(&self.rx).map_err(DeviceError::Malformed)? {
                Decoded::Frame { value, consumed } => {
                    self.rx.drain(..consumed.min(self.rx.len()));
                    return Ok(value);
                }
                Decoded::Incomplete => {}
            }
            let n = self.transport.read(&mut chunk, deadline).map_err(transport_error)?;
            if n == 0 {
                log::warn!("session: no complete reply after {} ms (limit {} ms, {} bytes buffered)", started.elapsed().as_millis(), timeout.as_millis(), self.rx.len());
                return Err(DeviceError::Timeout);
            }
            self.rx.extend_from_slice(&chunk[..n]);
        }
    }

    /// Sends a request and waits for its reply, repeating the whole exchange up
    /// to `retries` more times on a timeout or malformed reply.
    pub fn request<C: Codec>(
        &mut self,
        codec: &mut C,
        request: &C::Request,
        timeout: Duration,
        retries: u32,
    ) -> Result<C::Response, DeviceError> {
        let mut attempt = 0;
        loop {
            let bytes = codec.encode(request).map_err(DeviceError::Invalid)?;
            log::debug!("session: request {} timeout {} ms (attempt {} of {})", command_label(&bytes), timeout.as_millis(), attempt + 1, retries + 1);
            self.send(&bytes)?;
            match self.receive(codec, timeout) {
                Err(e) if e.is_retryable() && attempt < retries => {
                    log::warn!("session: retrying {} after: {e}", command_label(&bytes));
                    attempt += 1
                }
                result => return result,
            }
        }
    }

    /// Sends a command and reads until the line goes quiet (01-transport §3).
    pub fn request_until_idle(&mut self, bytes: &[u8], timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        log::debug!("session: request-until-idle {} timeout {} ms", command_label(bytes), timeout.as_millis());
        self.send(bytes)?;
        self.read_until_idle(timeout)
    }

    /// Like `request_until_idle`, but a silent device is fine: if nothing arrives within
    /// `idle + quiet_cap`, the reply is empty ("quiet") instead of a timeout. For commands
    /// whose reply is ignored, such as `.log:0` (01-transport §1.4).
    pub fn request_until_quiet(&mut self, bytes: &[u8], quiet_cap: Duration) -> Result<Vec<u8>, DeviceError> {
        let limit = Duration::from_millis(self.timeouts.idle_ms) + quiet_cap;
        log::debug!("session: request-until-quiet {} limit {} ms", command_label(bytes), limit.as_millis());
        self.send(bytes)?;
        match self.read_until_idle(limit) {
            Err(DeviceError::Timeout) => {
                log::info!("session: {} got no reply within {} ms, treating the line as quiet", command_label(bytes), limit.as_millis());
                Ok(Vec::new())
            }
            other => other,
        }
    }

    /// Reads until data has arrived and then `idle_ms` passes with nothing new.
    /// Fails with `Timeout` if nothing arrives, or the line never goes quiet, in time.
    pub fn read_until_idle(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        let started = Instant::now();
        let overall = started + timeout;
        let idle = Duration::from_millis(self.timeouts.idle_ms);
        let mut data = std::mem::take(&mut self.rx);
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            let quiet_at = Instant::now() + idle;
            let waiting_for_quiet = !data.is_empty() && quiet_at < overall;
            let deadline = if waiting_for_quiet { quiet_at } else { overall };
            let n = self.transport.read(&mut chunk, deadline).map_err(transport_error)?;
            if n == 0 {
                return if waiting_for_quiet {
                    log::debug!("session: idle after {} bytes, {} ms", data.len(), started.elapsed().as_millis());
                    Ok(data)
                } else {
                    log::warn!("session: nothing received within {} ms (limit {} ms, {} bytes so far)", started.elapsed().as_millis(), timeout.as_millis(), data.len());
                    Err(DeviceError::Timeout)
                };
            }
            data.extend_from_slice(&chunk[..n]);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(bytes) = self.farewell.take() {
            let _ = self.send(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;
    use crate::framing::{TextCodec, CR};
    use crate::transport::mock::MockTransport;

    const SHORT: Duration = Duration::from_millis(50);

    fn session(mock: MockTransport, kind: PortKind) -> Session {
        // The mock never waits, so only the order of deadlines matters: quiet comes first.
        let timeouts = Timeouts { idle_ms: 1, ..Timeouts::for_kind(kind) };
        Session::new(Box::new(mock), kind, timeouts)
    }

    #[test]
    fn explain_adds_a_hint_only_on_bluetooth_timeouts() {
        let bt = session(MockTransport::new(), PortKind::Bluetooth);
        let usb = session(MockTransport::new(), PortKind::Usb);
        assert!(bt.explain(&DeviceError::Timeout).contains("paired"));
        assert_eq!(usb.explain(&DeviceError::Timeout), DeviceError::Timeout.to_string());
    }

    #[test]
    fn bluetooth_waits_longer_than_usb() {
        let usb = Timeouts::for_kind(PortKind::Usb);
        let bt = Timeouts::for_kind(PortKind::Bluetooth);
        assert_eq!((usb.idle_ms, bt.idle_ms), (200, 1_000));
        assert_eq!((usb.config_read_ms, bt.config_read_ms), (30_000, 90_000));
        assert_eq!(usb.command_ms, bt.command_ms);
    }

    #[test]
    fn request_reassembles_a_split_reply() {
        for kind in [PortKind::Usb, PortKind::Bluetooth] {
            let mock = MockTransport::new().reply(&[b"<SEC", b"STAT>1,0,1", b",0,3\r"]);
            let log = mock.log();
            let mut s = session(mock, kind);
            let reply = s.request(&mut TextCodec::until(CR), ":sec_status", SHORT, 0).unwrap();
            assert_eq!(reply, "<SECSTAT>1,0,1,0,3");
            assert_eq!(log.writes(), vec![b":sec_status\r".to_vec()]);
        }
    }

    #[test]
    fn extra_bytes_serve_the_next_receive_but_not_the_next_command() {
        let mock = MockTransport::new().reply(&[b"<A>\r<B>:1\r"]).reply(&[b"fresh\r"]);
        let mut s = session(mock, PortKind::Usb);
        let mut codec = TextCodec::until(CR);
        assert_eq!(s.request(&mut codec, "x", SHORT, 0).unwrap(), "<A>");
        assert_eq!(s.receive(&mut codec, SHORT).unwrap(), "<B>:1");
        // Anything left over is discarded before the next command is written.
        let mock = MockTransport::new().reply(&[b"one\rstale\r"]).reply(&[b"fresh\r"]);
        let mut s = session(mock, PortKind::Usb);
        assert_eq!(s.request(&mut codec, "x", SHORT, 0).unwrap(), "one");
        assert_eq!(s.request(&mut codec, "y", SHORT, 0).unwrap(), "fresh");
    }

    #[test]
    fn retries_after_a_timeout() {
        let mock = MockTransport::new().silence().reply(&[b"ok\r"]);
        let log = mock.log();
        let mut s = session(mock, PortKind::Bluetooth);
        assert_eq!(s.request(&mut TextCodec::until(CR), "x", SHORT, 1).unwrap(), "ok");
        assert_eq!(log.writes().len(), 2);
    }

    #[test]
    fn gives_up_after_the_last_retry() {
        let mock = MockTransport::new().silence().silence().silence();
        let log = mock.log();
        let mut s = session(mock, PortKind::Usb);
        let err = s.request(&mut TextCodec::until(CR), "x", SHORT, 2).unwrap_err();
        assert_eq!(err, DeviceError::Timeout);
        assert_eq!(log.writes().len(), 3);
    }

    #[test]
    fn transport_failures_are_not_retried() {
        let mock = MockTransport::new().fail(ErrorKind::BrokenPipe);
        let log = mock.log();
        let mut s = session(mock, PortKind::Usb);
        let err = s.request(&mut TextCodec::until(CR), "x", SHORT, 5).unwrap_err();
        assert!(matches!(err, DeviceError::Transport(_)));
        assert_eq!(log.writes().len(), 1);
    }

    #[test]
    fn idle_read_collects_every_chunk() {
        let mock = MockTransport::new().reply(&[b"cfg_info:0:03.29\r", b"cfg_info:5:OK\r"]);
        let mut s = session(mock, PortKind::Usb);
        let reply = s.request_until_idle(b":cfg_info:?\r", SHORT).unwrap();
        assert_eq!(reply, b"cfg_info:0:03.29\rcfg_info:5:OK\r");
    }

    #[test]
    fn idle_read_times_out_on_silence() {
        let mut s = session(MockTransport::new().silence(), PortKind::Bluetooth);
        assert_eq!(s.request_until_idle(b"x\r", SHORT), Err(DeviceError::Timeout));
    }

    #[test]
    fn dropping_a_session_sends_the_farewell() {
        let mock = MockTransport::new();
        let log = mock.log();
        let mut s = session(mock, PortKind::Usb);
        s.farewell = Some(b":cfg_disconnect\r".to_vec());
        drop(s);
        assert_eq!(log.writes(), vec![b":cfg_disconnect\r".to_vec()]);
    }
}
