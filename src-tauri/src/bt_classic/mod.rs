//! Bluetooth Classic "Nearby Bluetooth" discovery (01-transport §2.2).
//!
//! The platform backends only report raw sightings ({address, name, class of device,
//! rssi, paired}). Everything the UI relies on (the tracker class-of-device filter,
//! de-duplication by address, the 1-minute expiry and the model hint parsed from the
//! name) is pure logic in this file and unit-tested.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod pair_macos;
#[cfg(target_os = "macos")]
pub mod rfcomm_macos;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod other;

/// One device report from a backend: an inquiry result or an OS-paired device.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub address: String,
    pub name: Option<String>,
    pub class_of_device: u32,
    pub rssi: Option<i16>,
    pub paired: bool,
}

/// A platform's Bluetooth Classic inquiry. Backends are polled from the UI thread.
pub trait ClassicScanner: Send {
    /// Starts (or restarts) one inquiry round, clearing the previous round's results.
    fn start_inquiry(&mut self) -> Result<(), String>;
    fn stop_inquiry(&mut self);
    /// Devices found by the inquiry that is running (or that last ran).
    fn found(&mut self) -> Result<Vec<Sighting>, String>;
    /// Devices paired in the OS, whether or not they are in range.
    fn paired(&mut self) -> Result<Vec<Sighting>, String>;
}

/// Seconds per inquiry round. When a round ends the next one starts (§2.2 step 5).
pub const INQUIRY_SECS: u64 = 10;
/// A device is listed while it was seen within this window (§2.2 step 4).
pub const EXPIRY_MS: u64 = 60_000;

/// Service-class bits of the CoD (bits 13–23) must be exactly Positioning | Audio |
/// Telephony = 0x308, i.e. `cod & 0xFFE000 == 0x610000` (§2.2 step 2). Major/minor bits
/// are not checked.
pub fn is_likely_tracker(class_of_device: u32) -> bool {
    class_of_device & 0xFF_E000 == 0x61_0000
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelHint {
    pub model: String,
    pub serial: String,
    /// Always "from Bluetooth name": this is our own heuristic, not a probed value.
    pub source: &'static str,
}

/// Our own heuristic (not from the spec), based on the owner's real tracker name
/// `FMB920_1234567`: `^([A-Z]{3}[A-Z0-9]{3,4})_(\d{6,8})$`.
pub fn model_hint(name: &str) -> Option<ModelHint> {
    let (model, serial) = name.split_once('_')?;
    let m = model.as_bytes();
    let ok_model = (6..=7).contains(&m.len())
        && m[..3].iter().all(u8::is_ascii_uppercase)
        && m[3..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let ok_serial = (6..=8).contains(&serial.len()) && serial.bytes().all(|c| c.is_ascii_digit());
    (ok_model && ok_serial).then(|| ModelHint { model: model.into(), serial: serial.into(), source: "from Bluetooth name" })
}

/// Canonical address form: upper-case hex pairs separated by ':'. Backends differ
/// (`aa-bb-…` on macOS, `AA:BB:…` on BlueZ).
pub fn normalize_address(addr: &str) -> String {
    let hex: String = addr.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_ascii_uppercase();
    if hex.len() != 12 {
        return addr.trim().to_ascii_uppercase();
    }
    hex.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(":")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyDevice {
    pub address: String,
    pub name: Option<String>,
    pub class_of_device: u32,
    pub rssi: Option<i16>,
    pub paired: bool,
    /// Class-of-device exact service-bit match (§2.2). A hint only.
    pub likely_tracker: bool,
    pub model_hint: Option<ModelHint>,
    /// Unix ms of the last inquiry sighting; None for a paired device not seen nearby.
    pub last_seen: Option<u64>,
}

/// De-duplicates sightings by address and expires them after [`EXPIRY_MS`].
#[derive(Default)]
pub struct Tracker {
    seen: HashMap<String, (Sighting, u64)>,
}

impl Tracker {
    pub fn observe(&mut self, mut s: Sighting, now_ms: u64) {
        s.address = normalize_address(&s.address);
        let key = s.address.clone();
        if let Some((old, _)) = self.seen.get(&key) {
            // Keep a name/rssi we already had if this report lacks one.
            s.name = s.name.or_else(|| old.name.clone());
            s.rssi = s.rssi.or(old.rssi);
            s.paired |= old.paired;
            if s.class_of_device == 0 {
                s.class_of_device = old.class_of_device;
            }
        }
        self.seen.insert(key, (s, now_ms));
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.seen.retain(|_, (_, t)| now_ms.saturating_sub(*t) <= EXPIRY_MS);
    }

    /// Recent inquiry sightings merged with the OS-paired list, trackers first.
    pub fn snapshot(&self, paired: &[Sighting], now_ms: u64) -> Vec<NearbyDevice> {
        let mut out: HashMap<String, NearbyDevice> = HashMap::new();
        for (s, t) in self.seen.values() {
            if now_ms.saturating_sub(*t) <= EXPIRY_MS {
                out.insert(s.address.clone(), device(s, Some(*t)));
            }
        }
        for p in paired {
            let addr = normalize_address(&p.address);
            match out.get_mut(&addr) {
                Some(d) => {
                    d.paired = true;
                    if d.name.is_none() {
                        d.name = p.name.clone();
                        d.model_hint = p.name.as_deref().and_then(model_hint);
                    }
                    if d.class_of_device == 0 && p.class_of_device != 0 {
                        d.class_of_device = p.class_of_device;
                        d.likely_tracker = is_likely_tracker(p.class_of_device);
                    }
                }
                None => {
                    let mut s = p.clone();
                    s.address = addr.clone();
                    s.paired = true;
                    out.insert(addr, device(&s, None));
                }
            }
        }
        let mut v: Vec<_> = out.into_values().collect();
        v.sort_by(|a, b| {
            b.likely_tracker
                .cmp(&a.likely_tracker)
                .then(b.last_seen.is_some().cmp(&a.last_seen.is_some()))
                .then(a.name.as_deref().unwrap_or("~").to_lowercase().cmp(&b.name.as_deref().unwrap_or("~").to_lowercase()))
                .then(a.address.cmp(&b.address))
        });
        v
    }
}

fn device(s: &Sighting, last_seen: Option<u64>) -> NearbyDevice {
    NearbyDevice {
        address: s.address.clone(),
        name: s.name.clone().filter(|n| !n.trim().is_empty()),
        class_of_device: s.class_of_device,
        rssi: s.rssi,
        paired: s.paired,
        likely_tracker: is_likely_tracker(s.class_of_device),
        model_hint: s.name.as_deref().and_then(model_hint),
        last_seen,
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Builds this OS's scanner, or explains why there is none.
fn platform_scanner() -> Result<Box<dyn ClassicScanner>, String> {
    #[cfg(target_os = "macos")]
    return Ok(Box::new(macos::MacScanner::default()));
    #[cfg(target_os = "linux")]
    return Ok(Box::new(linux::BluezScanner::default()));
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err(other::UNSUPPORTED.into());
}

struct Inner {
    scanner: Option<Box<dyn ClassicScanner>>,
    tracker: Tracker,
    scanning: bool,
    round_started_ms: u64,
}

pub struct BtClassicState(Mutex<Inner>);

impl Default for BtClassicState {
    fn default() -> Self {
        Self(Mutex::new(Inner { scanner: None, tracker: Tracker::default(), scanning: false, round_started_ms: 0 }))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub supported: bool,
    pub scanning: bool,
    pub message: Option<String>,
    pub devices: Vec<NearbyDevice>,
}

impl Inner {
    fn scanner(&mut self) -> Result<&mut Box<dyn ClassicScanner>, String> {
        if self.scanner.is_none() {
            self.scanner = Some(platform_scanner()?);
        }
        Ok(self.scanner.as_mut().unwrap())
    }

    fn snapshot(&mut self) -> Snapshot {
        let now = now_ms();
        let mut message = None;
        let supported = match self.scanner() {
            Ok(_) => true,
            Err(e) => {
                message = Some(e);
                false
            }
        };
        if supported && self.scanning {
            let restart = now.saturating_sub(self.round_started_ms) >= (INQUIRY_SECS + 1) * 1000;
            let sc = self.scanner().unwrap();
            let found = sc.found();
            let restarted = if restart { Some(sc.start_inquiry()) } else { None };
            match found {
                Ok(list) => list.into_iter().for_each(|s| {
                    log_sighting("inquiry", &s);
                    self.tracker.observe(s, now)
                }),
                Err(e) => {
                    log::warn!("bluetooth classic: reading inquiry results failed: {e}");
                    message = Some(e)
                }
            }
            match restarted {
                Some(Ok(())) => {
                    log::debug!("bluetooth classic: inquiry round restarted");
                    self.round_started_ms = now
                }
                Some(Err(e)) => {
                    log::error!("bluetooth classic: inquiry restart failed: {e}");
                    self.scanning = false;
                    message = Some(e);
                }
                None => {}
            }
        }
        self.tracker.expire(now);
        let paired = if supported { self.scanner().unwrap().paired().unwrap_or_else(|e| {
            log::warn!("bluetooth classic: listing paired devices failed: {e}");
            message.get_or_insert(e);
            Vec::new()
        }) } else { Vec::new() };
        paired.iter().for_each(|s| log_sighting("paired", s));
        Snapshot { supported, scanning: self.scanning, message, devices: self.tracker.snapshot(&paired, now) }
    }
}

// Commands are sync on purpose: Tauri runs sync commands on the main thread, which is
// where IOBluetooth delivers inquiry results (main run loop).

/// Logs each device address once per kind of sighting (name, address, class of device).
fn log_sighting(kind: &str, s: &Sighting) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let key = format!("{kind}/{}/{:?}", s.address, s.name);
    if SEEN.get_or_init(Default::default).lock().unwrap().insert(key) {
        log::debug!(
            "bluetooth classic: {kind} device {:?} {} CoD 0x{:06x} paired {} rssi {:?}",
            s.name.as_deref().unwrap_or(""), s.address, s.class_of_device, s.paired, s.rssi
        );
    }
}

#[tauri::command]
pub fn bt_classic_start(state: State<'_, BtClassicState>) -> Result<Snapshot, String> {
    let mut g = state.0.lock().unwrap();
    log::info!("bluetooth classic: inquiry start ({INQUIRY_SECS} s rounds)");
    g.scanner()?.start_inquiry().inspect_err(|e| log::error!("bluetooth classic: inquiry start failed: {e}"))?;
    g.scanning = true;
    g.round_started_ms = now_ms();
    Ok(g.snapshot())
}

#[tauri::command]
pub fn bt_classic_stop(state: State<'_, BtClassicState>) -> Snapshot {
    let mut g = state.0.lock().unwrap();
    log::info!("bluetooth classic: inquiry stop");
    if let Ok(s) = g.scanner() {
        s.stop_inquiry();
    }
    g.scanning = false;
    g.snapshot()
}

/// Called about once a second by the panel: collects results, restarts finished rounds,
/// expires old sightings and returns the merged list.
#[tauri::command]
pub fn bt_classic_poll(state: State<'_, BtClassicState>) -> Snapshot {
    state.0.lock().unwrap().snapshot()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResult {
    pub state: String,
    pub address: Option<String>,
    pub message: Option<String>,
}

/// Starts in-app pairing with `pin` (macOS only). Stops the inquiry first: pairing and
/// an inquiry compete for the radio. Poll [`bt_classic_pair_status`] for the outcome.
#[tauri::command]
pub fn bt_classic_pair(state: State<'_, BtClassicState>, address: String, pin: String) -> Result<PairResult, String> {
    // The PIN is never logged.
    log::info!("bluetooth classic: pairing requested for {address} (PIN of {} digits supplied)", pin.len());
    {
        let mut g = state.0.lock().unwrap();
        if let Ok(s) = g.scanner() {
            s.stop_inquiry();
        }
        g.scanning = false;
    }
    #[cfg(target_os = "macos")]
    {
        let s = pair_macos::start(&address, &pin).inspect_err(|e| log::error!("bluetooth classic: pairing could not start: {e}"))?;
        log::info!("bluetooth classic: pairing state {}", s.state);
        Ok(PairResult { state: s.state.into(), address: s.address, message: s.message })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (address, pin);
        Err("In-app pairing is only available on macOS. Pair in your OS Bluetooth settings.".into())
    }
}

#[tauri::command]
pub fn bt_classic_pair_status() -> PairResult {
    #[cfg(target_os = "macos")]
    {
        let s = pair_macos::status();
        PairResult { state: s.state.into(), address: s.address, message: s.message }
    }
    #[cfg(not(target_os = "macos"))]
    PairResult { state: "idle".into(), address: None, message: None }
}

/// Opens the OS Bluetooth settings so the user can pair (the tool itself never pairs, §2.3).
#[tauri::command]
pub fn bt_classic_open_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.BluetoothSettings").status();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg("ms-settings:bluetooth").status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r: std::io::Result<std::process::ExitStatus> =
        Err(std::io::Error::other("Open your desktop's Bluetooth settings (or use bluetoothctl) to pair"));
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(addr: &str, name: Option<&str>, cod: u32, paired: bool) -> Sighting {
        Sighting { address: addr.into(), name: name.map(Into::into), class_of_device: cod, rssi: None, paired }
    }

    #[test]
    fn cod_filter_is_exact_service_bits() {
        assert!(is_likely_tracker(0x61_0000));
        assert!(is_likely_tracker(0x61_1F00)); // major/minor bits ignored
        assert!(is_likely_tracker(0x61_00FC));
        assert!(!is_likely_tracker(0x61_2000)); // limited-discoverable bit 13 set
        assert!(!is_likely_tracker(0x41_0000)); // telephony missing
        assert!(!is_likely_tracker(0x71_0000)); // extra service bit
        assert!(!is_likely_tracker(0x24_0404)); // audio headset
        assert!(!is_likely_tracker(0));
    }

    #[test]
    fn model_hint_from_name() {
        let h = model_hint("FMB920_1234567").unwrap();
        assert_eq!((h.model.as_str(), h.serial.as_str(), h.source), ("FMB920", "1234567", "from Bluetooth name"));
        assert_eq!(model_hint("FMC1301_123456").unwrap().model, "FMC1301");
        assert_eq!(model_hint("FMB920_12345678").unwrap().serial, "12345678");
        for bad in ["FMB920", "FMB920_12345", "FMB920_123456789", "fmb920_1234567", "FM9920_1234567", "FMB92_1234567", "FMB92000_1234567", "FMB920_96010a4", "FMB920_960_1074", "MIBOX4 nown", "[TV] Samsung TV", ""] {
            assert!(model_hint(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn address_normalization() {
        assert_eq!(normalize_address("a4-c1-38-0b-11-22"), "A4:C1:38:0B:11:22");
        assert_eq!(normalize_address("A4:C1:38:0B:11:22"), "A4:C1:38:0B:11:22");
        assert_eq!(normalize_address("weird"), "WEIRD");
    }

    #[test]
    fn dedup_by_address_keeps_latest_and_merges() {
        let mut t = Tracker::default();
        t.observe(s("a4-c1-38-0b-11-22", Some("FMB920_1234567"), 0x61_0000, false), 1_000);
        t.observe(s("A4:C1:38:0B:11:22", None, 0, false), 5_000);
        let v = t.snapshot(&[], 5_000);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name.as_deref(), Some("FMB920_1234567"));
        assert!(v[0].likely_tracker);
        assert_eq!(v[0].last_seen, Some(5_000));
    }

    #[test]
    fn expiry_after_one_minute() {
        let mut t = Tracker::default();
        t.observe(s("00:11:22:33:44:55", Some("old"), 0, false), 0);
        t.observe(s("00:11:22:33:44:66", Some("new"), 0, false), 30_000);
        assert_eq!(t.snapshot(&[], 60_000).len(), 2); // exactly 1 min: still listed
        assert_eq!(t.snapshot(&[], 60_001).len(), 1);
        t.expire(90_001);
        assert!(t.snapshot(&[], 90_001).is_empty());
    }

    #[test]
    fn paired_devices_are_merged_and_listed_when_absent() {
        let mut t = Tracker::default();
        t.observe(s("00:11:22:33:44:55", Some("MIBOX4 nown"), 0x08_0414, false), 0);
        let paired = [s("00-11-22-33-44-55", Some("MIBOX4 nown"), 0x08_0414, true), s("aa-bb-cc-dd-ee-ff", Some("FMB920_1234567"), 0x61_0000, true)];
        let v = t.snapshot(&paired, 1_000);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].address, "AA:BB:CC:DD:EE:FF"); // tracker first
        assert!(v[0].paired && v[0].last_seen.is_none());
        assert!(v[1].paired && v[1].last_seen == Some(0) && !v[1].likely_tracker);
    }
}
