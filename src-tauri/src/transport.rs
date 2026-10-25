//! Byte transports a tracker can be reached over. USB (CDC) and Bluetooth
//! Classic (SPP, once paired) both show up as serial ports and carry the same
//! byte stream (docs/spec/01-transport.md §2), so one serial transport serves both.

use std::io::{self, ErrorKind, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serialport::{ClearBuffer, SerialPort};

use crate::ports::PortKind;

pub trait Transport: Send {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Reads up to `buf.len()` bytes, returning as soon as any arrive.
    /// Returns `Ok(0)` only once `deadline` has passed with nothing received.
    fn read(&mut self, buf: &mut [u8], deadline: Instant) -> io::Result<usize>;

    /// Drops anything received but not yet read, e.g. a late reply before a retry.
    fn clear_input(&mut self) -> io::Result<()>;
}

// Serial line settings from docs/spec/01-transport.md §1.2. The OS receive buffer
// size (1 MiB there) can't be set through the serialport crate; we drain the port
// continuously instead.
pub const DEFAULT_BAUD_RATE: u32 = 115_200;
pub const DEFAULT_DATA_BITS: u8 = 8;
pub const DEFAULT_PARITY: Parity = Parity::None;
pub const DEFAULT_STOP_BITS: u8 = 1;
pub const DEFAULT_FLOW_CONTROL: FlowControl = FlowControl::None;
pub const DEFAULT_DTR: bool = true;
pub const DEFAULT_RTS: bool = true;
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 500;
pub const DEFAULT_WRITE_TIMEOUT_MS: u64 = 500;

/// How a port is opened: USB retries any failure (01-transport §1.3); Bluetooth
/// waits briefly first and retries only "in use" / "invalid argument" (§2.1, §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPolicy {
    pub attempts: u32,
    pub retry_delay: Duration,
    pub initial_delay: Duration,
    /// Retry only when the failure was "in use" or "invalid argument".
    pub only_busy_errors: bool,
}

impl OpenPolicy {
    pub fn for_kind(kind: PortKind) -> Self {
        match kind {
            PortKind::Bluetooth => Self {
                attempts: 3,
                retry_delay: Duration::from_millis(100),
                initial_delay: Duration::from_millis(100),
                only_busy_errors: true,
            },
            _ => Self {
                attempts: 4,
                retry_delay: Duration::from_millis(300),
                initial_delay: Duration::ZERO,
                only_busy_errors: false,
            },
        }
    }
}

struct OpenFailure {
    message: String,
    busy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

/// Serial line settings, sent by the frontend; missing fields take the defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SerialSettings {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow_control: FlowControl,
    /// `None` leaves the line untouched (Bluetooth ports, 01-transport §2.1).
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
    /// Upper bound for one low-level read; whole operations have their own timeouts.
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            baud_rate: DEFAULT_BAUD_RATE,
            data_bits: DEFAULT_DATA_BITS,
            parity: DEFAULT_PARITY,
            stop_bits: DEFAULT_STOP_BITS,
            flow_control: DEFAULT_FLOW_CONTROL,
            dtr: Some(DEFAULT_DTR),
            rts: Some(DEFAULT_RTS),
            read_timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            write_timeout_ms: DEFAULT_WRITE_TIMEOUT_MS,
        }
    }
}

impl SerialSettings {
    /// Line settings for a port kind. Bluetooth serial ports have no real line:
    /// DTR/RTS are left alone and flow control is off. The OS API still needs some
    /// baud number; the Bluetooth driver ignores it (01-transport §2.1, §2.4).
    pub fn for_kind(kind: PortKind) -> Self {
        match kind {
            PortKind::Bluetooth => Self { dtr: None, rts: None, ..Self::default() },
            _ => Self::default(),
        }
    }

    fn read_timeout(&self) -> Duration {
        Duration::from_millis(self.read_timeout_ms.max(1))
    }

    fn write_timeout(&self) -> Duration {
        Duration::from_millis(self.write_timeout_ms.max(1))
    }

    fn builder(&self, port: &str) -> Result<serialport::SerialPortBuilder, String> {
        let data_bits = match self.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            8 => serialport::DataBits::Eight,
            other => return Err(format!("unsupported data bits: {other}")),
        };
        let stop_bits = match self.stop_bits {
            1 => serialport::StopBits::One,
            2 => serialport::StopBits::Two,
            other => return Err(format!("unsupported stop bits: {other}")),
        };
        let parity = match self.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        };
        let flow_control = match self.flow_control {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Software => serialport::FlowControl::Software,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
        };
        let builder = serialport::new(port, self.baud_rate)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .parity(parity)
            .flow_control(flow_control)
            .timeout(self.read_timeout());
        Ok(match self.dtr {
            Some(on) => builder.dtr_on_open(on),
            None => builder.preserve_dtr_on_open(),
        })
    }
}

pub struct SerialTransport {
    port: Box<dyn SerialPort>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl SerialTransport {
    /// Opens the port, retrying as `policy` says (01-transport §1.3, §2.1).
    pub fn open(port: &str, settings: &SerialSettings, policy: OpenPolicy) -> Result<Self, String> {
        log::info!(
            "serial: opening {port}: {} baud, {}{:?}{}, flow {:?}, dtr {:?}, rts {:?}, read/write timeout {}/{} ms; policy {} attempts, {} ms initial delay",
            settings.baud_rate, settings.data_bits, settings.parity, settings.stop_bits, settings.flow_control,
            settings.dtr, settings.rts, settings.read_timeout_ms, settings.write_timeout_ms, policy.attempts, policy.initial_delay.as_millis()
        );
        thread::sleep(policy.initial_delay);
        let mut last_error = String::new();
        for attempt in 1..=policy.attempts {
            match Self::open_once(port, settings) {
                Ok(transport) => {
                    log::info!("serial: {port} open (attempt {attempt})");
                    return Ok(transport);
                }
                Err(failure) => {
                    log::warn!("serial: open attempt {attempt} of {} failed: {} (busy: {})", policy.attempts, failure.message, failure.busy);
                    last_error = failure.message;
                    if policy.only_busy_errors && !failure.busy {
                        break;
                    }
                }
            }
            if attempt < policy.attempts {
                thread::sleep(policy.retry_delay);
            }
        }
        Err(format!("could not open {port}: {last_error}"))
    }

    fn open_once(port: &str, settings: &SerialSettings) -> Result<Self, OpenFailure> {
        let plain = |message: String| OpenFailure { message, busy: false };
        let mut serial = settings.builder(port).map_err(plain)?.open().map_err(|e| {
            let invalid_input = e.kind() == serialport::ErrorKind::InvalidInput;
            let message = describe_open_error(port, e);
            OpenFailure { busy: invalid_input || message == BUSY_MESSAGE, message }
        })?;
        // With hardware flow control the driver owns RTS.
        if let (Some(rts), false) = (settings.rts, settings.flow_control == FlowControl::Hardware) {
            serial
                .write_request_to_send(rts)
                .map_err(|e| plain(format!("could not set RTS: {e}")))?;
        }
        Ok(Self {
            port: serial,
            read_timeout: settings.read_timeout(),
            write_timeout: settings.write_timeout(),
        })
    }
}

/// serialport reports both "missing" and "held by another program" as NoDevice;
/// if the OS still lists the port, it's the latter (01-transport §1.3: "port in use").
const BUSY_MESSAGE: &str = "port in use";

fn describe_open_error(port: &str, error: serialport::Error) -> String {
    let listed = || {
        serialport::available_ports()
            .map(|ports| ports.iter().any(|p| p.port_name == port))
            .unwrap_or(false)
    };
    if error.kind() == serialport::ErrorKind::NoDevice && listed() {
        BUSY_MESSAGE.into()
    } else {
        error.to_string()
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        log::info!("serial: closing {}", self.port.name().unwrap_or_default());
    }
}

impl Transport for SerialTransport {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.set_timeout(self.write_timeout)?;
        self.port.write_all(bytes)?;
        self.port.flush()
    }

    fn read(&mut self, buf: &mut [u8], deadline: Instant) -> io::Result<usize> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || buf.is_empty() {
                return Ok(0);
            }
            self.port.set_timeout(remaining.min(self.read_timeout))?;
            match self.port.read(buf) {
                Ok(0) => thread::sleep(Duration::from_millis(1)),
                Ok(n) => return Ok(n),
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock) => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn clear_input(&mut self) -> io::Result<()> {
        self.port.clear(ClearBuffer::Input).map_err(io::Error::from)
    }
}

#[cfg(test)]
pub mod mock {
    //! A scripted transport: each write is answered by the next scripted reply.

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;

    enum Reply {
        Chunks(Vec<Vec<u8>>),
        Silence,
        Fail(ErrorKind),
    }

    /// Shared view of every write, still readable after the transport is boxed.
    #[derive(Clone, Default)]
    pub struct WriteLog(Arc<Mutex<Vec<Vec<u8>>>>);

    impl WriteLog {
        pub fn writes(&self) -> Vec<Vec<u8>> {
            self.0.lock().unwrap().clone()
        }
    }

    #[derive(Default)]
    pub struct MockTransport {
        script: VecDeque<Reply>,
        /// Chunks the "device" has sent; each read returns at most one.
        incoming: VecDeque<Vec<u8>>,
        log: WriteLog,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self::default()
        }

        /// The next write is answered with these chunks, one per read.
        pub fn reply(mut self, chunks: &[&[u8]]) -> Self {
            self.script
                .push_back(Reply::Chunks(chunks.iter().map(|c| c.to_vec()).collect()));
            self
        }

        /// The next write gets no answer, so the read times out.
        pub fn silence(mut self) -> Self {
            self.script.push_back(Reply::Silence);
            self
        }

        /// The next write fails, as if the port went away.
        pub fn fail(mut self, kind: ErrorKind) -> Self {
            self.script.push_back(Reply::Fail(kind));
            self
        }

        pub fn log(&self) -> WriteLog {
            self.log.clone()
        }
    }

    impl Transport for MockTransport {
        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.log.0.lock().unwrap().push(bytes.to_vec());
            match self.script.pop_front() {
                Some(Reply::Chunks(chunks)) => self.incoming.extend(chunks),
                Some(Reply::Fail(kind)) => return Err(io::Error::new(kind, "mock write failed")),
                Some(Reply::Silence) | None => {}
            }
            Ok(())
        }

        /// Never waits: an empty queue stands for "deadline passed".
        fn read(&mut self, buf: &mut [u8], _deadline: Instant) -> io::Result<usize> {
            let Some(mut chunk) = self.incoming.pop_front() else {
                return Ok(0);
            };
            if chunk.len() > buf.len() {
                self.incoming.push_front(chunk.split_off(buf.len()));
            }
            buf[..chunk.len()].copy_from_slice(&chunk);
            Ok(chunk.len())
        }

        fn clear_input(&mut self) -> io::Result<()> {
            self.incoming.clear();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockTransport;
    use super::*;

    #[test]
    fn settings_fill_missing_fields_with_defaults() {
        let settings: SerialSettings =
            serde_json::from_str(r#"{"baudRate":9600,"parity":"even"}"#).unwrap();
        assert_eq!(settings.baud_rate, 9600);
        assert_eq!(settings.parity, Parity::Even);
        assert_eq!(settings.data_bits, DEFAULT_DATA_BITS);
        assert_eq!(settings.read_timeout_ms, DEFAULT_READ_TIMEOUT_MS);
        assert_eq!((settings.dtr, settings.rts), (Some(true), Some(true)));
    }

    #[test]
    fn usb_settings_assert_dtr_and_rts() {
        let s = SerialSettings::for_kind(PortKind::Usb);
        assert_eq!((s.dtr, s.rts), (Some(true), Some(true)));
        assert_eq!(s, SerialSettings::for_kind(PortKind::Other));
    }

    #[test]
    fn bluetooth_settings_leave_dtr_and_rts_alone() {
        let s = SerialSettings::for_kind(PortKind::Bluetooth);
        assert_eq!((s.dtr, s.rts), (None, None));
        assert_eq!(s.flow_control, FlowControl::None);
    }

    #[test]
    fn explicit_null_means_untouched_line() {
        let s: SerialSettings = serde_json::from_str(r#"{"dtr":null}"#).unwrap();
        assert_eq!((s.dtr, s.rts), (None, Some(true)));
    }

    #[test]
    fn open_policy_per_kind() {
        let usb = OpenPolicy::for_kind(PortKind::Usb);
        assert_eq!((usb.attempts, usb.retry_delay), (4, Duration::from_millis(300)));
        assert!(!usb.only_busy_errors);
        let bt = OpenPolicy::for_kind(PortKind::Bluetooth);
        assert_eq!((bt.attempts, bt.retry_delay), (3, Duration::from_millis(100)));
        assert_eq!(bt.initial_delay, Duration::from_millis(100));
        assert!(bt.only_busy_errors);
    }

    #[test]
    fn rejects_unsupported_line_settings() {
        let bad_bits = SerialSettings { data_bits: 9, ..Default::default() };
        assert!(bad_bits.builder("x").is_err());
        let bad_stop = SerialSettings { stop_bits: 3, ..Default::default() };
        assert!(bad_stop.builder("x").is_err());
    }

    #[test]
    fn mock_splits_chunks_to_fit_the_buffer() {
        let mut mock = MockTransport::new().reply(&[b"abcdef"]);
        mock.write_all(b"q").unwrap();
        let mut buf = [0u8; 4];
        let deadline = Instant::now();
        assert_eq!(mock.read(&mut buf, deadline).unwrap(), 4);
        assert_eq!(&buf, b"abcd");
        assert_eq!(mock.read(&mut buf, deadline).unwrap(), 2);
        assert_eq!(&buf[..2], b"ef");
        assert_eq!(mock.read(&mut buf, deadline).unwrap(), 0);
        assert_eq!(mock.log().writes(), vec![b"q".to_vec()]);
    }
}
