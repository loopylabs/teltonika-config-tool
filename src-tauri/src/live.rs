//! Live data: the debug log stream (03-commands `.log:1` / `.log:0`, §7.7) and status
//! polling (03-commands §7.6: 0x0002 / 0x0102, a count frame, then that many data frames).
//!
//! The engine is plain synchronous code over a `Session`, so it is tested with
//! `MockTransport`. `lib.rs` runs it in one background worker that leases the
//! session only between user operations (see `Connection::lease_live`).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::device::{DeviceError, Session};
use crate::fmbx::{frame_count, parse_status_data, FmbxCodec, Request, StatusRecord, StatusValue, RESPONSE_ACK};
use crate::framing::ascii;
use crate::ports::PortKind;

pub const GET_STATUS: u16 = 0x0002;
pub const STATUS_DATA: u16 = 0x0003;
pub const GET_EXTENDED: u16 = 0x0102;
pub const EXTENDED_DATA: u16 = 0x0103;
/// Group mask sent with 0x0102: all groups (03 §7.3).
const ALL_GROUPS: [u8; 8] = [0, 0, 0, 0, 0, 0, 0xFF, 0xFF];
/// Feature bit of status 784 that enables 0x0102 (03 §7.6.1).
const FEATURE_EXTENDED: u64 = 0x800;
/// Back-off after a failed cycle (03 §7.6.1).
pub const FAILURE_BACKOFF: Duration = Duration::from_secs(30);

/// Longest the log reader blocks before it re-checks for work.
const LOG_SLICE: Duration = Duration::from_millis(200);

/// Default poll interval: the reference tool waits 5 s after each cycle (03 §7.6.1), on
/// both links; the user can pick another.
pub fn default_interval_ms(_kind: PortKind) -> u64 {
    5_000
}

/// Per-frame read timeout, the same on USB and Bluetooth (03 §7.6.2).
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// Milliseconds since the Unix epoch, taken when the line was received.
    pub at_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusField {
    pub id: u32,
    pub label: String,
    /// Card group: Protocol, CAN, or Other (not defined by the spec).
    pub group: &'static str,
    pub display: String,
    pub raw_hex: String,
    pub type_code: u8,
    /// False: the spec doesn't define this ID, so only raw data is shown (TODO(spec)).
    pub defined: bool,
    pub unit: Option<&'static str>,
    /// Numeric value, when the record is a number. Coordinates outside their valid range
    /// give none.
    pub num: Option<f64>,
    /// Not a device-status ID (outside 256..=826): an I/O element record.
    pub io: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusEvent {
    pub kind: PortKind,
    pub at_ms: u64,
    /// Round-trip time of the last successful poll.
    pub rtt_ms: Option<u64>,
    /// Failed polls in a row (timeouts or bad replies).
    pub failures: u32,
    pub ok: bool,
    /// Polling has switched to 0x0102 (feature bit 0x800).
    pub extended: bool,
    pub fields: Vec<StatusField>,
}

pub enum Out {
    Log(Vec<LogLine>),
    Status(StatusEvent),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Want {
    pub log: bool,
    pub poll: Option<Duration>,
}

impl Want {
    pub fn idle(&self) -> bool {
        !self.log && self.poll.is_none()
    }
}

#[derive(Default)]
pub struct Engine {
    partial: Vec<u8>,
    /// The next cycle starts at this time (set when a cycle finishes).
    next_due: Option<Instant>,
    /// Sticky once status 784 has feature bit 0x800 (03 §7.6.1).
    extended: bool,
    failures: u32,
    last_rtt: Option<u64>,
    /// Fixed request ID for tests.
    pub fixed_id: Option<u32>,
    /// Wait after a failed cycle; tests shorten it.
    pub backoff: Option<Duration>,
}

impl Engine {
    /// One slice of work. Returns whether the link was used (the caller sleeps if not).
    /// Polls never overlap: the next one is due `interval` after the previous finished.
    pub fn step(&mut self, s: &mut Session, kind: PortKind, want: &Want, out: &mut dyn FnMut(Out)) -> Result<bool, DeviceError> {
        let due = want.poll.map(|_| self.next_due.is_none_or(|t| Instant::now() >= t));
        if due == Some(true) {
            // Log text and binary frames share the link: stop the log for the poll.
            if s.log_streaming {
                let rest = s.stop_log()?;
                self.feed(&rest, out);
                self.flush(out);
            }
            let wait = self.poll(s, kind, want.poll.unwrap_or_default(), out)?;
            self.next_due = Some(Instant::now() + wait);
        } else if want.log {
            if !s.log_streaming {
                log::info!("log stream: starting");
                s.start_log()?;
            }
            let mut wait = LOG_SLICE;
            if let (Some(_), Some(t)) = (want.poll, self.next_due) {
                wait = wait.min(t.saturating_duration_since(Instant::now()));
            }
            let data = s.read_available(wait)?;
            if data.is_empty() {
                self.flush(out);
                return Ok(false);
            }
            self.feed(&data, out);
        } else if s.log_streaming {
            // Stop requested: always send .log:0 and keep what was still in flight.
            log::info!("log stream: stop requested");
            let rest = s.stop_log()?;
            self.feed(&rest, out);
            self.flush(out);
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// One status cycle. Returns how long to wait before the next: the interval after a
    /// success, the back-off after a failure (a lost link is an error instead).
    fn poll(&mut self, s: &mut Session, kind: PortKind, interval: Duration, out: &mut dyn FnMut(Out)) -> Result<Duration, DeviceError> {
        let started = Instant::now();
        let result = self.cycle(s);
        let rtt = started.elapsed().as_millis() as u64;
        match result {
            Ok(records) => {
                log::debug!("status poll: ok, {} records in {rtt} ms (extended {})", records.len(), self.extended);
                self.failures = 0;
                self.last_rtt = Some(rtt);
                let fields = records.iter().map(field).collect();
                out(Out::Status(StatusEvent { kind, at_ms: now_ms(), rtt_ms: Some(rtt), failures: 0, ok: true, extended: self.extended, fields }));
                Ok(interval)
            }
            Err(DeviceError::Transport(m)) => {
                log::error!("status poll: link lost: {m}");
                Err(DeviceError::Transport(m))
            }
            // Any bad frame fails the whole exchange; its records are discarded.
            Err(e) => {
                self.failures += 1;
                log::warn!("status poll: cycle failed after {rtt} ms ({e}); failure {} in a row", self.failures);
                out(Out::Status(StatusEvent { kind, at_ms: now_ms(), rtt_ms: self.last_rtt, failures: self.failures, ok: false, extended: self.extended, fields: vec![] }));
                Ok(self.backoff.unwrap_or(FAILURE_BACKOFF))
            }
        }
    }

    /// 0x0002 until status 784 shows feature 0x800, then (in the same cycle, and for good)
    /// 0x0102 with all groups.
    fn cycle(&mut self, s: &mut Session) -> Result<Vec<StatusRecord>, DeviceError> {
        let id = self.fixed_id;
        if !self.extended {
            let records = status_exchange(s, id, GET_STATUS, vec![], STATUS_DATA, false)?;
            let features = records.iter().find_map(|r| if r.id == 784 { as_u64(&r.value) } else { None });
            if features.is_none_or(|f| f & FEATURE_EXTENDED == 0) {
                return Ok(records);
            }
            self.extended = true;
        }
        status_exchange(s, id, GET_EXTENDED, ALL_GROUPS.to_vec(), EXTENDED_DATA, true)
    }

    /// Splits received bytes into lines (CR, LF or CRLF); a trailing partial line waits.
    fn feed(&mut self, data: &[u8], out: &mut dyn FnMut(Out)) {
        self.partial.extend_from_slice(data);
        let at = now_ms();
        let mut lines = Vec::new();
        while let Some(i) = self.partial.iter().position(|&b| b == b'\r' || b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            let text = ascii(&line[..line.len() - 1]);
            if !text.trim().is_empty() {
                lines.push(LogLine { at_ms: at, text });
            }
        }
        if !lines.is_empty() {
            log::trace!("log stream: {} lines", lines.len());
            out(Out::Log(lines));
        }
    }

    /// The line went quiet: an unterminated line is complete enough to show.
    fn flush(&mut self, out: &mut dyn FnMut(Out)) {
        if self.partial.is_empty() {
            return;
        }
        let text = ascii(&std::mem::take(&mut self.partial));
        if !text.trim().is_empty() {
            out(Out::Log(vec![LogLine { at_ms: now_ms(), text }]));
        }
    }
}

// ---------------------------------------------------------------- status exchange

fn as_u64(v: &StatusValue) -> Option<u64> {
    match v {
        StatusValue::Unsigned(v) => Some(*v),
        _ => None,
    }
}

/// One exchange of 03 §7.6.2: send the request, read the count frame (response 0x0001),
/// then that many data frames (`data_id`) and concatenate their records. A bad frame
/// (checksum, wrong response ID, another request ID, silence for 30 s) fails everything.
/// `send` stops the debug log first, so no log text can mix into the replies (§7.7).
pub fn status_exchange(s: &mut Session, fixed_id: Option<u32>, command: u16, payload: Vec<u8>, data_id: u16, wide_ids: bool) -> Result<Vec<StatusRecord>, DeviceError> {
    let mut codec = FmbxCodec::with_id(fixed_id);
    let bytes = crate::framing::Codec::encode(&mut codec, &Request { command, payload }).map_err(DeviceError::Invalid)?;
    s.send(&bytes)?;
    let count = s.receive(&mut codec, FRAME_TIMEOUT)?;
    if count.id != RESPONSE_ACK {
        return Err(DeviceError::Malformed(format!("expected a count frame, got response {:#06x}", count.id)));
    }
    let mut records = Vec::new();
    for _ in 0..frame_count(&count.payload) {
        let frame = s.receive(&mut codec, FRAME_TIMEOUT)?;
        if frame.id != data_id {
            return Err(DeviceError::Malformed(format!("expected response {data_id:#06x}, got {:#06x}", frame.id)));
        }
        records.extend(parse_status_data(&frame.payload, wide_ids));
    }
    Ok(records)
}

// ---------------------------------------------------------------- status decoding

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// Device-status IDs the spec names (03 §7.1, §7.6.3, §7.6.4): label, card group, unit.
/// Enumerated states (256, 257, 512, 513, 515, 516) stay raw numbers: their texts are not
/// in the spec (open question 70).
fn known(id: u32) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    Some(match id {
        256 => ("Fix state", "Position", None),
        257 => ("GNSS module state", "Position", None),
        258 => ("Fix time", "Position", Some("s")),
        265 => ("Latitude", "Position", Some("°")),
        266 => ("Longitude", "Position", Some("°")),
        512 => ("Modem state", "GSM", None),
        513 => ("SIM state", "GSM", None),
        515 => ("Signal level", "GSM", None),
        516 => ("Data-link state", "GSM", None),
        544 => ("Data sent", "GSM", Some("B")),
        545 => ("Data received", "GSM", Some("B")),
        546 => ("SMS sent", "GSM", None),
        547 => ("SMS received", "GSM", None),
        770 => ("Firmware version", "Identity", None),
        771 => ("Device time", "Time", None),
        772 => ("Uptime", "Time", Some("s")),
        773 => ("External voltage", "Power", Some("mV")),
        774 => ("Battery voltage", "Power", Some("mV")),
        775 => ("Battery level", "Power", None),
        776 => ("Battery charge state", "Power", None),
        781 => ("Binary protocol version", "Identity", None),
        784 => ("Binary feature mask", "Identity", None),
        787 => ("LV-CAN adapter name", "CAN", None),
        788 => ("LV-CAN adapter ID", "CAN", None),
        790 => ("LV-CAN present", "CAN", None),
        803 => ("Chip ID", "CAN", None),
        810 => ("Additional feature bits", "Identity", None),
        _ => return None,
    })
}

/// Device-status IDs are 256..=826 (03 §7.6.4); anything else is an I/O element.
fn is_status_id(id: u32) -> bool {
    (256..=826).contains(&id)
}

fn field(r: &StatusRecord) -> StatusField {
    let raw_hex = match &r.value {
        StatusValue::Unsigned(v) => format!("0x{v:X}"),
        StatusValue::Signed(v) => format!("{v}"),
        StatusValue::Float(v) => format!("{v}"),
        StatusValue::Text(t) => t.chars().map(|c| format!("{:02X}", c as u32 as u8)).collect::<Vec<_>>().join(" "),
        StatusValue::Bytes(b) => hex(b),
    };
    let named = known(r.id);
    let mut num = match &r.value {
        StatusValue::Unsigned(v) => Some(*v as f64),
        StatusValue::Signed(v) => Some(*v as f64),
        StatusValue::Float(v) if v.is_finite() => Some(*v),
        _ => None,
    };
    // Coordinates are valid only inside their range (03 §7.6.4).
    let limit = match r.id {
        265 => Some(90.0),
        266 => Some(180.0),
        _ => None,
    };
    if let (Some(limit), Some(v)) = (limit, num) {
        if v.abs() > limit {
            num = None;
        }
    }
    let version = |major: u64, minor: u64| format!("{major}.{minor}");
    let display = match (r.id, &r.value) {
        (781, StatusValue::Unsigned(v)) => version((v >> 8) & 0xFF, v & 0xFF),
        (781, StatusValue::Bytes(b)) if b.len() >= 2 => version(b[0] as u64, b[1] as u64),
        (790, StatusValue::Unsigned(v)) => (if *v != 0 { "Yes" } else { "No" }).to_string(),
        (784 | 810, StatusValue::Unsigned(v)) => format!("0x{v:X}"),
        (265 | 266, _) if num.is_none() => "no valid position".to_string(),
        (265 | 266, StatusValue::Float(v)) => format!("{v:.7}"),
        (_, StatusValue::Unsigned(v)) => v.to_string(),
        (_, StatusValue::Text(t)) if named.is_some() || !is_status_id(r.id) => t.clone(),
        (_, StatusValue::Float(v)) => v.to_string(),
        (_, StatusValue::Signed(v)) => v.to_string(),
        _ => raw_hex.clone(),
    };
    StatusField {
        id: r.id,
        label: named.map_or_else(|| format!("Status #{}", r.id), |n| n.0.to_string()),
        group: named.map_or("Other", |n| n.1),
        display,
        raw_hex,
        type_code: r.type_code,
        defined: named.is_some(),
        unit: named.and_then(|n| n.2),
        num,
        io: !is_status_id(r.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Timeouts;
    use crate::fmbx::{encode_frame, Frame};
    use crate::transport::mock::{MockTransport, WriteLog};

    fn session(mock: MockTransport, kind: PortKind) -> Session {
        let mut t = Timeouts::for_kind(kind);
        t.idle_ms = 1;
        Session::new(Box::new(mock), kind, t)
    }

    fn writes(log: &WriteLog) -> Vec<String> {
        log.writes().iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect()
    }

    fn frame_bytes(id: u16, payload: &[u8]) -> Vec<u8> {
        encode_frame(&Frame { request_id: TEST_ID, id, payload: payload.to_vec() }).unwrap()
    }

    /// Count frame (1) plus one 0x0003 data frame, echoing the fixed test request ID.
    fn status_reply(payload: &[u8]) -> Vec<u8> {
        [frame_bytes(RESPONSE_ACK, &[1]), frame_bytes(STATUS_DATA, payload)].concat()
    }

    fn fields_of(payload: &[u8]) -> Vec<StatusField> {
        parse_status_data(payload, false).iter().map(field).collect()
    }

    /// Request command IDs of every write that is a binary frame.
    fn commands(log: &WriteLog) -> Vec<u16> {
        log.writes().iter().filter(|w| w.starts_with(b"FMBX")).map(|w| u16::from_be_bytes([w[8], w[9]])).collect()
    }

    const TEST_ID: u32 = 7;

    fn engine() -> Engine {
        Engine { fixed_id: Some(TEST_ID), backoff: Some(Duration::ZERO), ..Default::default() }
    }

    const PAYLOAD: &[u8] = &[0, 0x03, 0x0D, 3, 0, 1, 0x03, 0x10, 5, 0, 0, 0, 5, 0, 3, 16, 4, b'T', b'E', b'S', b'T'];

    fn want_poll() -> Want {
        Want { log: false, poll: Some(Duration::ZERO) }
    }

    fn run(e: &mut Engine, s: &mut Session, kind: PortKind, want: &Want) -> (Result<bool, DeviceError>, Vec<Out>) {
        let mut outs = Vec::new();
        let r = e.step(s, kind, want, &mut |o| outs.push(o));
        (r, outs)
    }

    fn status_of(outs: &[Out]) -> &StatusEvent {
        outs.iter().find_map(|o| if let Out::Status(s) = o { Some(s) } else { None }).expect("status event")
    }

    fn log_texts(outs: &[Out]) -> Vec<String> {
        outs.iter().flat_map(|o| if let Out::Log(l) = o { l.iter().map(|x| x.text.clone()).collect() } else { vec![] }).collect()
    }

    #[test]
    fn decodes_the_spec_example() {
        let f = fields_of(PAYLOAD);
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].id, f[0].display.as_str(), f[0].defined), (781, "0.1", true));
        assert_eq!(f[1].label, "Binary feature mask");
        assert_eq!(f[1].display, "0x5");
        assert_eq!((f[2].id, f[2].defined, f[2].group, f[2].io), (3, false, "Other", true));
        assert_eq!(f[2].display, "TEST");
        assert_eq!(f[2].raw_hex, "54 45 53 54");
    }

    #[test]
    fn polls_over_usb_and_bluetooth() {
        for kind in [PortKind::Usb, PortKind::Bluetooth] {
            let mock = MockTransport::new().reply(&[&status_reply(PAYLOAD)]);
            let mut s = session(mock, kind);
            let (r, outs) = run(&mut engine(), &mut s, kind, &want_poll());
            assert_eq!(r, Ok(true));
            let ev = status_of(&outs);
            assert!(ev.ok && ev.rtt_ms.is_some());
            assert_eq!((ev.kind, ev.fields.len(), ev.failures), (kind, 3, 0));
        }
    }

    #[test]
    fn slow_bluetooth_reply_in_pieces_still_decodes() {
        let frame = status_reply(PAYLOAD);
        let (a, b) = frame.split_at(7);
        let mut s = session(MockTransport::new().reply(&[a, b]), PortKind::Bluetooth);
        let (_, outs) = run(&mut engine(), &mut s, PortKind::Bluetooth, &want_poll());
        assert!(status_of(&outs).ok);
    }

    #[test]
    fn timeouts_are_counted_and_reset_by_a_success() {
        let mock = MockTransport::new().silence().silence().reply(&[&status_reply(PAYLOAD)]);
        let mut s = session(mock, PortKind::Bluetooth);
        let mut e = engine();
        for n in 1..=2 {
            let (r, outs) = run(&mut e, &mut s, PortKind::Bluetooth, &want_poll());
            assert_eq!(r, Ok(true), "a timeout is not a lost link");
            let ev = status_of(&outs);
            assert!(!ev.ok && ev.fields.is_empty());
            assert_eq!(ev.failures, n);
        }
        let (_, outs) = run(&mut e, &mut s, PortKind::Bluetooth, &want_poll());
        assert_eq!(status_of(&outs).failures, 0);
    }

    #[test]
    fn dropped_link_is_an_error_not_a_failure_count() {
        let mock = MockTransport::new().fail(std::io::ErrorKind::BrokenPipe);
        let mut s = session(mock, PortKind::Bluetooth);
        let (r, _) = run(&mut engine(), &mut s, PortKind::Bluetooth, &want_poll());
        assert!(matches!(r, Err(DeviceError::Transport(_))));
    }

    #[test]
    fn polls_wait_for_the_interval() {
        let mock = MockTransport::new().reply(&[&status_reply(PAYLOAD)]);
        let mut s = session(mock, PortKind::Usb);
        let mut e = engine();
        let want = Want { log: false, poll: Some(Duration::from_secs(60)) };
        assert_eq!(run(&mut e, &mut s, PortKind::Usb, &want).0, Ok(true));
        let (r, outs) = run(&mut e, &mut s, PortKind::Usb, &want);
        assert_eq!(r, Ok(false));
        assert!(outs.is_empty());
    }

    #[test]
    fn log_lines_are_split_and_stamped() {
        let mock = MockTransport::new().reply(&[b"GSM: ok\r\nGPS: fi", b"x\r\n"]);
        let mut s = session(mock, PortKind::Usb);
        let mut e = engine();
        let want = Want { log: true, poll: None };
        let mut texts = vec![];
        for _ in 0..3 {
            let (_, outs) = run(&mut e, &mut s, PortKind::Usb, &want);
            texts.extend(log_texts(&outs));
        }
        assert_eq!(texts, ["GSM: ok", "GPS: fix"]);
        assert!(s.log_streaming);
    }

    #[test]
    fn stopping_sends_log_off_and_keeps_late_lines() {
        let mock = MockTransport::new().reply(&[]).reply(&[b"late line\r"]);
        let wl = mock.log();
        let mut s = session(mock, PortKind::Usb);
        let mut e = engine();
        let _ = run(&mut e, &mut s, PortKind::Usb, &Want { log: true, poll: None });
        let (_, outs) = run(&mut e, &mut s, PortKind::Usb, &Want::default());
        assert_eq!(log_texts(&outs), ["late line"]);
        assert!(!s.log_streaming);
        assert_eq!(writes(&wl), [".log:1\r", ".log:0\r"]);
    }

    #[test]
    fn polling_pauses_the_log_and_the_reply_has_no_log_text() {
        // .log:1 is answered with a line; at the poll, .log:0 drains a late line and
        // the status request gets a clean frame.
        let mock = MockTransport::new().reply(&[b"log A\r\n"]).reply(&[b"log B\r\n"]).reply(&[&status_reply(PAYLOAD)]);
        let wl = mock.log();
        let mut s = session(mock, PortKind::Usb);
        let mut e = engine();
        let log_only = Want { log: true, poll: Some(Duration::from_secs(60)) };
        e.next_due = Some(Instant::now() + Duration::from_secs(60));
        let (_, o1) = run(&mut e, &mut s, PortKind::Usb, &log_only);
        assert_eq!(log_texts(&o1), ["log A"]);
        e.next_due = None;
        let (_, o2) = run(&mut e, &mut s, PortKind::Usb, &log_only);
        assert_eq!(log_texts(&o2), ["log B"]);
        assert!(status_of(&o2).ok);
        assert!(!s.log_streaming);
        assert_eq!(writes(&wl)[..2], [".log:1\r", ".log:0\r"]);
    }

    #[test]
    fn user_commands_stop_the_log_first() {
        let mock = MockTransport::new().reply(&[b"noise\r"]).reply(&[b"junk\r"]).reply(&[b"<OK>\r"]);
        let wl = mock.log();
        let mut s = session(mock, PortKind::Usb);
        s.start_log().unwrap();
        s.send(b":cfg_info:?\r").unwrap();
        assert!(!s.log_streaming);
        assert_eq!(writes(&wl), [".log:1\r", ".log:0\r", ":cfg_info:?\r"]);
    }

    #[test]
    fn data_frames_are_concatenated_after_the_count() {
        let a = [&[0u8][..], &[0x03, 0x0D, 3, 0, 1]].concat();
        let b = [&[0u8][..], &[0x03, 0x10, 5, 0, 0, 0, 5]].concat();
        let reply = [frame_bytes(RESPONSE_ACK, &[0, 2]), frame_bytes(STATUS_DATA, &a), frame_bytes(STATUS_DATA, &b)].concat();
        let mut s = session(MockTransport::new().reply(&[&reply]), PortKind::Usb);
        let (_, outs) = run(&mut engine(), &mut s, PortKind::Usb, &want_poll());
        let ev = status_of(&outs);
        assert!(ev.ok, "{ev:?}");
        assert_eq!(ev.fields.iter().map(|f| f.id).collect::<Vec<_>>(), [781, 784]);
    }

    #[test]
    fn any_bad_frame_fails_the_exchange_and_backs_off() {
        let a = [&[0u8][..], &[0x03, 0x0D, 3, 0, 1]].concat();
        let mut bad = frame_bytes(STATUS_DATA, &a);
        let last = bad.len() - 1;
        bad[last] ^= 0xFF; // checksum
        let cases: Vec<Vec<u8>> = vec![
            [frame_bytes(RESPONSE_ACK, &[2]), frame_bytes(STATUS_DATA, &a), bad].concat(), // second frame bad CRC
            [frame_bytes(RESPONSE_ACK, &[2]), frame_bytes(STATUS_DATA, &a)].concat(),      // second frame missing
            [frame_bytes(RESPONSE_ACK, &[1]), frame_bytes(0x0103, &a)].concat(),           // wrong response ID
            frame_bytes(STATUS_DATA, &a),                                                  // no count frame first
        ];
        for reply in cases {
            let mut s = session(MockTransport::new().reply(&[&reply]), PortKind::Usb);
            let mut e = Engine { fixed_id: Some(TEST_ID), ..Default::default() };
            let (r, outs) = run(&mut e, &mut s, PortKind::Usb, &Want { log: false, poll: Some(Duration::ZERO) });
            assert_eq!(r, Ok(true));
            let ev = status_of(&outs);
            assert!(!ev.ok && ev.fields.is_empty() && ev.failures == 1);
            // Back-off is 30 s: the next step does nothing.
            let (r, outs) = run(&mut e, &mut s, PortKind::Usb, &Want { log: false, poll: Some(Duration::ZERO) });
            assert_eq!((r, outs.len()), (Ok(false), 0));
        }
    }

    #[test]
    fn switches_to_extended_status_for_good() {
        // 784 = 0x0800: feature bit 0x800.
        let basic = [&[0u8][..], &[0x03, 0x10, 5, 0, 0, 0x08, 0x00]].concat();
        let wide = [&[0u8][..], &[0, 0, 0x01, 0x09, 3, 0x31, 0x38]].concat(); // 265 as u16, just a record
        let ext = [frame_bytes(RESPONSE_ACK, &[0, 1]), frame_bytes(EXTENDED_DATA, &wide)].concat();
        let mock = MockTransport::new().reply(&[&status_reply(&basic)]).reply(&[&ext]).reply(&[&ext]);
        let wl = mock.log();
        let mut s = session(mock, PortKind::Bluetooth);
        let mut e = engine();
        let (_, outs) = run(&mut e, &mut s, PortKind::Bluetooth, &want_poll());
        // Same cycle: 0x0002 then 0x0102, and the 0x0102 records are shown.
        assert_eq!(commands(&wl), [0x0002, 0x0102]);
        let ev = status_of(&outs);
        assert!(ev.ok && ev.extended);
        assert_eq!(ev.fields.iter().map(|f| f.id).collect::<Vec<_>>(), [265]);
        let sent = &wl.writes()[1];
        assert_eq!(&sent[12..20], &[0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
        // Later cycles send only 0x0102.
        let _ = run(&mut e, &mut s, PortKind::Bluetooth, &want_poll());
        assert_eq!(commands(&wl), [0x0002, 0x0102, 0x0102]);
    }

    #[test]
    fn stays_basic_without_the_feature_bit() {
        let mock = MockTransport::new().reply(&[&status_reply(PAYLOAD)]).reply(&[&status_reply(PAYLOAD)]);
        let wl = mock.log();
        let mut s = session(mock, PortKind::Usb);
        let mut e = engine();
        let _ = run(&mut e, &mut s, PortKind::Usb, &want_poll());
        let _ = run(&mut e, &mut s, PortKind::Usb, &want_poll());
        assert_eq!(commands(&wl), [0x0002, 0x0002]);
    }

    #[test]
    fn next_cycle_starts_an_interval_after_the_previous_finished() {
        let mut s = session(MockTransport::new().reply(&[&status_reply(PAYLOAD)]), PortKind::Usb);
        let mut e = engine();
        let before = Instant::now();
        let _ = run(&mut e, &mut s, PortKind::Usb, &Want { log: false, poll: Some(Duration::from_secs(5)) });
        let due = e.next_due.unwrap();
        assert!(due >= before + Duration::from_secs(5) && due <= Instant::now() + Duration::from_secs(5));
    }

    #[test]
    fn frame_timeout_is_30_seconds_on_both_links() {
        assert_eq!(FRAME_TIMEOUT, Duration::from_secs(30));
        assert_eq!(FAILURE_BACKOFF, Duration::from_secs(30));
        assert_eq!(default_interval_ms(PortKind::Usb), default_interval_ms(PortKind::Bluetooth));
    }

    #[test]
    fn telemetry_fields_decode() {
        // 773 u16 12600, 771 u32 1.7e9, 265 float64 54.6872 (as a 2-byte-ID record), 266 float64 200 (invalid).
        let lat = 54.6872f64.to_be_bytes();
        let lon = 200.0f64.to_be_bytes();
        let payload = [
            &[0u8][..],
            &[0x03, 0x05, 3, 0x31, 0x38],
            &[0x03, 0x03, 5, 0x65, 0x53, 0xF1, 0x00],
            &[0x01, 0x09, 9][..], &lat,
            &[0x01, 0x0A, 9][..], &lon,
            &[0x03, 0x02, 4, 0xFF, 0xFF, 0xFF, 0xFE], // 770 as i32 -2
        ]
        .concat();
        let f = fields_of(&payload);
        let by = |id: u32| f.iter().find(|x| x.id == id).unwrap();
        assert_eq!((by(773).num, by(773).unit, by(773).group), (Some(12600.0), Some("mV"), "Power"));
        assert_eq!(by(771).num, Some(1_700_000_000.0));
        assert_eq!((by(265).num, by(265).display.as_str()), (Some(54.6872), "54.6872000"));
        assert_eq!((by(266).num, by(266).display.as_str()), (None, "no valid position"));
        assert_eq!(by(770).num, Some(-2.0));
        assert!(f.iter().all(|x| !x.io));
    }
}
