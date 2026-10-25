//! Bluetooth LE discovery. Our own feature, not part of the reference tool: it only lists nearby
//! advertisers so the user can see what is around. Configuration over BLE is not supported
//! (spec 01 v2); a tracker is configured through its paired Bluetooth serial port.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use btleplug::api::{Central, CentralEvent, CentralState, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use futures::stream::{Stream, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Notify;

pub const DEFAULT_TIMEOUT_SECS: u64 = 15;
const MAX_TIMEOUT_SECS: u64 = 300;

/// One advertisement as seen by the scanner.
#[derive(Clone, Debug, PartialEq)]
pub struct Advert {
    pub id: String,
    /// Real Bluetooth address; absent where the OS hides it (macOS gives only an opaque id).
    pub address: Option<String>,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    pub manufacturer_data: Vec<(u16, Vec<u8>)>,
    pub services: Vec<String>,
}

/// A device row as sent to the UI.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleDevice {
    pub id: String,
    pub address: Option<String>,
    pub name: Option<String>,
    pub rssi: Option<i16>,
    /// Signal bars 0-4 derived from `rssi`.
    pub bars: u8,
    /// Hex of each manufacturer record as `company id (4 hex digits) : data`.
    pub manufacturer_data: Vec<String>,
    pub services: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub last_seen: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScanState {
    Idle,
    Scanning,
    BluetoothOff,
    PermissionDenied,
    NoAdapter,
    Unsupported,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStatus {
    pub state: ScanState,
    pub message: Option<String>,
}

impl ScanStatus {
    fn of(state: ScanState) -> Self {
        let message = match state {
            ScanState::Idle | ScanState::Scanning => None,
            ScanState::BluetoothOff => Some("Bluetooth is turned off. Turn it on in your system settings and scan again."),
            ScanState::PermissionDenied => Some(if cfg!(target_os = "macos") {
                "Bluetooth access was denied. Allow it in System Settings > Privacy & Security > Bluetooth, then scan again."
            } else {
                "Bluetooth access was denied. Check your system's Bluetooth permissions."
            }),
            ScanState::NoAdapter => Some("No Bluetooth adapter was found on this computer."),
            ScanState::Unsupported => Some("Bluetooth LE scanning is not supported on this system."),
            ScanState::Error => None,
        };
        Self { state, message: message.map(String::from) }
    }

    fn error(text: String) -> Self {
        Self { state: ScanState::Error, message: Some(text) }
    }
}

/// Maps a scanner failure to a user-facing state.
#[derive(Debug, Clone, PartialEq)]
pub enum BleProblem {
    BluetoothOff,
    PermissionDenied,
    NoAdapter,
    Unsupported,
    Other(String),
}

impl BleProblem {
    fn status(self) -> ScanStatus {
        match self {
            Self::BluetoothOff => ScanStatus::of(ScanState::BluetoothOff),
            Self::PermissionDenied => ScanStatus::of(ScanState::PermissionDenied),
            Self::NoAdapter => ScanStatus::of(ScanState::NoAdapter),
            Self::Unsupported => ScanStatus::of(ScanState::Unsupported),
            Self::Other(text) => ScanStatus::error(text),
        }
    }
}

impl From<btleplug::Error> for BleProblem {
    fn from(e: btleplug::Error) -> Self {
        use btleplug::Error as E;
        match e {
            E::PermissionDenied => Self::PermissionDenied,
            E::NoAdapterAvailable => Self::NoAdapter,
            E::NotSupported(_) => Self::Unsupported,
            other => {
                // Platform layers report a powered-off radio as a generic error.
                let text = other.to_string();
                let lower = text.to_lowercase();
                if lower.contains("powered off") || lower.contains("not powered") || lower.contains("bluetooth is off") {
                    Self::BluetoothOff
                } else if lower.contains("permission") || lower.contains("not authorized") || lower.contains("unauthorized") {
                    Self::PermissionDenied
                } else {
                    Self::Other(text)
                }
            }
        }
    }
}

pub type AdvertStream = Pin<Box<dyn Stream<Item = Advert> + Send>>;
type BoxFut<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Hardware seam: the dedup and timing logic in `run_scan` is tested against a fake.
pub trait Scanner: Send + Sync + 'static {
    fn start(&self) -> BoxFut<'_, Result<AdvertStream, BleProblem>>;
    fn stop(&self) -> BoxFut<'_, ()>;
}

/// Keeps one row per device id and reports which advertisements change what the UI shows.
#[derive(Default)]
pub struct DeviceTable {
    devices: HashMap<String, BleDevice>,
}

impl DeviceTable {
    /// Merges an advertisement. Returns the row to send when the device is new or its RSSI, name,
    /// manufacturer data or services changed; an identical repeat only refreshes `last_seen`
    /// silently. Fields missing from a later advertisement keep their earlier value, since
    /// scanners deliver name, data and RSSI in separate events.
    pub fn update(&mut self, advert: Advert, now_ms: u64) -> Option<BleDevice> {
        match self.devices.get_mut(&advert.id) {
            None => {
                let device = BleDevice {
                    id: advert.id.clone(),
                    address: advert.address,
                    name: advert.name.filter(|n| !n.is_empty()),
                    rssi: advert.rssi,
                    bars: rssi_bars(advert.rssi),
                    manufacturer_data: hex_records(&advert.manufacturer_data),
                    services: advert.services,
                    last_seen: now_ms,
                };
                self.devices.insert(advert.id, device.clone());
                Some(device)
            }
            Some(known) => {
                let mut changed = false;
                if advert.rssi.is_some() && advert.rssi != known.rssi {
                    known.rssi = advert.rssi;
                    known.bars = rssi_bars(advert.rssi);
                    changed = true;
                }
                if let Some(name) = advert.name.filter(|n| !n.is_empty()) {
                    if known.name.as_ref() != Some(&name) {
                        known.name = Some(name);
                        changed = true;
                    }
                }
                if known.address.is_none() && advert.address.is_some() {
                    known.address = advert.address;
                    changed = true;
                }
                let records = hex_records(&advert.manufacturer_data);
                if !records.is_empty() && records != known.manufacturer_data {
                    known.manufacturer_data = records;
                    changed = true;
                }
                if !advert.services.is_empty() && advert.services != known.services {
                    known.services = advert.services;
                    changed = true;
                }
                known.last_seen = now_ms;
                changed.then(|| known.clone())
            }
        }
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }
}

fn hex_records(records: &[(u16, Vec<u8>)]) -> Vec<String> {
    let mut out: Vec<String> = records
        .iter()
        .map(|(company, data)| {
            let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
            format!("{company:04X}:{hex}")
        })
        .collect();
    out.sort();
    out
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Runs one scan until the timeout, `stop` or the stream ends. Emits through `sink`; returns the
/// final status. Always asks the scanner to stop on the way out.
pub async fn run_scan<S: Scanner + ?Sized>(
    scanner: &S,
    timeout: Duration,
    stop: &Notify,
    sink: &(dyn Fn(ScanEvent) + Send + Sync),
    clock: &(dyn Fn() -> u64 + Send + Sync),
) -> ScanStatus {
    log::info!("ble: scan starting, timeout {} s", timeout.as_secs());
    let mut stream = match scanner.start().await {
        Ok(stream) => stream,
        Err(problem) => {
            let status = problem.status();
            log::warn!("ble: scan could not start: {:?}", status.state);
            sink(ScanEvent::State(status.clone()));
            return status;
        }
    };
    sink(ScanEvent::State(ScanStatus::of(ScanState::Scanning)));
    let mut table = DeviceTable::default();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = stop.notified() => break,
            next = stream.next() => match next {
                Some(advert) => {
                    if let Some(device) = table.update(advert, clock()) {
                        log::trace!("ble: device update {:?}", device.name);
                        sink(ScanEvent::Device(device));
                    }
                }
                None => break,
            },
        }
    }
    drop(stream);
    scanner.stop().await;
    log::info!("ble: scan ended, {} devices seen", table.len());
    let status = ScanStatus::of(ScanState::Idle);
    sink(ScanEvent::State(status.clone()));
    status
}

pub enum ScanEvent {
    Device(BleDevice),
    State(ScanStatus),
}

// ---------------------------------------------------------------- btleplug backend

struct BtleScanner {
    adapter: tokio::sync::Mutex<Option<Adapter>>,
}

impl BtleScanner {
    fn new() -> Self {
        Self { adapter: tokio::sync::Mutex::new(None) }
    }
}

async fn open_adapter() -> Result<Adapter, BleProblem> {
    let manager = Manager::new().await?;
    let adapter = manager.adapters().await?.into_iter().next().ok_or(BleProblem::NoAdapter)?;
    if let Ok(CentralState::PoweredOff) = adapter.adapter_state().await {
        return Err(BleProblem::BluetoothOff);
    }
    Ok(adapter)
}

async fn advert_for(adapter: &Adapter, event: CentralEvent) -> Option<Advert> {
    let id = match event {
        CentralEvent::DeviceDiscovered(id)
        | CentralEvent::DeviceUpdated(id)
        | CentralEvent::RssiUpdate { id, .. }
        | CentralEvent::ManufacturerDataAdvertisement { id, .. }
        | CentralEvent::ServiceDataAdvertisement { id, .. }
        | CentralEvent::ServicesAdvertisement { id, .. } => id,
        _ => return None,
    };
    let peripheral = adapter.peripheral(&id).await.ok()?;
    let props = peripheral.properties().await.ok()??;
    let address = props.address.to_string();
    let hidden = props.address.into_inner() == [0; 6];
    let mut data: Vec<(u16, Vec<u8>)> = props.manufacturer_data.into_iter().collect();
    data.sort();
    Some(Advert {
        id: id.to_string(),
        address: (!hidden).then_some(address),
        name: props.local_name.or(props.advertisement_name),
        rssi: props.rssi,
        manufacturer_data: data,
        services: props.services.iter().map(|u| u.to_string()).collect(),
    })
}

impl Scanner for BtleScanner {
    fn start(&self) -> BoxFut<'_, Result<AdvertStream, BleProblem>> {
        Box::pin(async move {
            let adapter = open_adapter().await?;
            let events = adapter.events().await?;
            adapter.start_scan(ScanFilter::default()).await?;
            *self.adapter.lock().await = Some(adapter.clone());
            let stream = events.filter_map(move |event| {
                let adapter = adapter.clone();
                async move { advert_for(&adapter, event).await }
            });
            Ok(Box::pin(stream) as AdvertStream)
        })
    }

    fn stop(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            if let Some(adapter) = self.adapter.lock().await.take() {
                let _ = adapter.stop_scan().await;
            }
        })
    }
}

// ---------------------------------------------------------------- commands

#[derive(Default)]
pub struct BleState {
    /// Stop signal of the running scan, if any.
    running: Mutex<Option<Arc<Notify>>>,
}

#[tauri::command]
pub async fn start_ble_scan(
    app: AppHandle,
    state: State<'_, BleState>,
    timeout_secs: Option<u64>,
) -> Result<(), String> {
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).clamp(1, MAX_TIMEOUT_SECS));
    let stop = Arc::new(Notify::new());
    {
        let mut running = state.running.lock().unwrap();
        if running.is_some() {
            return Ok(());
        }
        *running = Some(stop.clone());
    }
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let scanner = BtleScanner::new();
        let sink_app = task_app.clone();
        let sink = move |event: ScanEvent| {
            let _ = match event {
                ScanEvent::Device(d) => sink_app.emit("ble-device", d),
                ScanEvent::State(s) => sink_app.emit("ble-scan-state", s),
            };
        };
        run_scan(&scanner, timeout, &stop, &sink, &now_ms).await;
        use tauri::Manager as _;
        *task_app.state::<BleState>().running.lock().unwrap() = None;
    });
    Ok(())
}

#[tauri::command]
pub fn stop_ble_scan(state: State<'_, BleState>) {
    if let Some(stop) = state.running.lock().unwrap().as_ref() {
        stop.notify_one();
    }
}

/// Signal bars (0-4) for an RSSI in dBm. Missing RSSI shows no bars.
pub fn rssi_bars(rssi: Option<i16>) -> u8 {
    match rssi {
        None => 0,
        Some(r) if r >= -60 => 4,
        Some(r) if r >= -70 => 3,
        Some(r) if r >= -80 => 2,
        Some(r) if r >= -90 => 1,
        Some(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn advert(id: &str, rssi: Option<i16>) -> Advert {
        Advert { id: id.into(), address: None, name: None, rssi, manufacturer_data: vec![], services: vec![] }
    }

    #[test]
    fn new_device_is_reported_once() {
        let mut t = DeviceTable::default();
        assert!(t.update(advert("a", Some(-60)), 1).is_some());
        assert!(t.update(advert("a", Some(-60)), 2).is_none());
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn rssi_change_updates_row_and_last_seen() {
        let mut t = DeviceTable::default();
        t.update(advert("a", Some(-60)), 1);
        let d = t.update(advert("a", Some(-75)), 5).unwrap();
        assert_eq!((d.rssi, d.last_seen), (Some(-75), 5));
    }

    #[test]
    fn missing_fields_keep_earlier_values() {
        let mut t = DeviceTable::default();
        let mut first = advert("a", Some(-60));
        first.name = Some("Tracker".into());
        first.manufacturer_data = vec![(0x1234, vec![0xAB, 0x01])];
        t.update(first, 1);
        let d = t.update(advert("a", Some(-61)), 2).unwrap();
        assert_eq!(d.name.as_deref(), Some("Tracker"));
        assert_eq!(d.manufacturer_data, vec!["1234:AB01".to_string()]);
        assert!(t.update(advert("a", None), 3).is_none());
    }

    #[test]
    fn name_arriving_later_is_reported() {
        let mut t = DeviceTable::default();
        t.update(advert("a", Some(-60)), 1);
        let mut named = advert("a", Some(-60));
        named.name = Some("X".into());
        assert_eq!(t.update(named, 2).unwrap().name.as_deref(), Some("X"));
    }

    #[test]
    fn empty_name_is_unnamed() {
        let mut t = DeviceTable::default();
        let mut a = advert("a", None);
        a.name = Some(String::new());
        assert_eq!(t.update(a, 1).unwrap().name, None);
    }

    #[test]
    fn devices_are_separate_by_id() {
        let mut t = DeviceTable::default();
        t.update(advert("a", Some(-60)), 1);
        assert!(t.update(advert("b", Some(-60)), 1).is_some());
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn bars_mapping() {
        assert_eq!(rssi_bars(None), 0);
        assert_eq!(rssi_bars(Some(-40)), 4);
        assert_eq!(rssi_bars(Some(-60)), 4);
        assert_eq!(rssi_bars(Some(-61)), 3);
        assert_eq!(rssi_bars(Some(-70)), 3);
        assert_eq!(rssi_bars(Some(-80)), 2);
        assert_eq!(rssi_bars(Some(-90)), 1);
        assert_eq!(rssi_bars(Some(-91)), 0);
    }

    struct Fake {
        adverts: StdMutex<Option<Vec<Advert>>>,
        problem: Option<BleProblem>,
        stopped: StdMutex<bool>,
        /// Keep the stream open after the adverts, so only the timeout or stop ends the scan.
        hang: bool,
    }

    impl Fake {
        fn new(adverts: Vec<Advert>, hang: bool) -> Self {
            Self { adverts: StdMutex::new(Some(adverts)), problem: None, stopped: StdMutex::new(false), hang }
        }
    }

    impl Scanner for Fake {
        fn start(&self) -> BoxFut<'_, Result<AdvertStream, BleProblem>> {
            let result = match &self.problem {
                Some(p) => Err(p.clone()),
                None => {
                    let items = self.adverts.lock().unwrap().take().unwrap();
                    let s = futures::stream::iter(items);
                    Ok(if self.hang {
                        Box::pin(s.chain(futures::stream::pending())) as AdvertStream
                    } else {
                        Box::pin(s) as AdvertStream
                    })
                }
            };
            Box::pin(async move { result })
        }
        fn stop(&self) -> BoxFut<'_, ()> {
            *self.stopped.lock().unwrap() = true;
            Box::pin(async {})
        }
    }

    fn collect(events: &StdMutex<Vec<String>>) -> impl Fn(ScanEvent) + Send + Sync + '_ {
        move |e| {
            let text = match e {
                ScanEvent::Device(d) => format!("device:{}:{:?}", d.id, d.rssi),
                ScanEvent::State(s) => format!("state:{:?}", s.state),
            };
            events.lock().unwrap().push(text);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn scan_dedups_and_times_out() {
        let fake = Fake::new(vec![advert("a", Some(-60)), advert("a", Some(-60)), advert("a", Some(-50)), advert("b", None)], true);
        let events = StdMutex::new(vec![]);
        let stop = Notify::new();
        let status = run_scan(&fake, Duration::from_secs(15), &stop, &collect(&events), &|| 0).await;
        assert_eq!(status.state, ScanState::Idle);
        assert_eq!(
            *events.lock().unwrap(),
            ["state:Scanning", "device:a:Some(-60)", "device:a:Some(-50)", "device:b:None", "state:Idle"]
        );
        assert!(*fake.stopped.lock().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn scan_stops_on_request() {
        let fake = Fake::new(vec![], true);
        let events = StdMutex::new(vec![]);
        let stop = Notify::new();
        stop.notify_one();
        run_scan(&fake, Duration::from_secs(3600), &stop, &collect(&events), &|| 0).await;
        assert_eq!(*events.lock().unwrap(), ["state:Scanning", "state:Idle"]);
    }

    #[tokio::test]
    async fn problems_become_states() {
        let mut fake = Fake::new(vec![], false);
        fake.problem = Some(BleProblem::BluetoothOff);
        let events = StdMutex::new(vec![]);
        let status = run_scan(&fake, Duration::from_secs(1), &Notify::new(), &collect(&events), &|| 0).await;
        assert_eq!(status.state, ScanState::BluetoothOff);
        assert!(status.message.unwrap().contains("turned off"));
        assert_eq!(*events.lock().unwrap(), ["state:BluetoothOff"]);
    }

    #[test]
    fn btleplug_errors_map_to_states() {
        assert_eq!(BleProblem::from(btleplug::Error::PermissionDenied), BleProblem::PermissionDenied);
        assert_eq!(BleProblem::from(btleplug::Error::NoAdapterAvailable), BleProblem::NoAdapter);
        assert_eq!(BleProblem::from(btleplug::Error::NotSupported("x".into())), BleProblem::Unsupported);
    }
}
