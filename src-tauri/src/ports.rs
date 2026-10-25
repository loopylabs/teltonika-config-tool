//! Serial ports a tracker could be connected on: USB (CDC) or a paired
//! Bluetooth Classic device exposed as a serial port (docs/spec/01-transport.md).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serialport::SerialPortType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PortKind {
    Usb,
    Bluetooth,
    Other,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub name: String,
    pub kind: PortKind,
    /// USB VID:PID matches 01-transport §1.1. A hint only: any port may be tried.
    pub likely_tracker: bool,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

/// USB VID:PID pairs of tracker virtual COM ports (01-transport §1.1). Some are
/// composite devices where only interface MI_00 is the serial port; the serialport
/// crate doesn't expose the interface number without an extra feature, so every
/// port of a matching device is flagged.
const TRACKER_USB_IDS: [(u16, u16); 5] = [
    (0x0E8D, 0x0023),
    (0x0483, 0x5740),
    (0x1FC9, 0x0094),
    (0x0483, 0x5702),
    (0x1D12, 0x0200),
];

pub fn is_tracker_usb_id(vid: u16, pid: u16) -> bool {
    TRACKER_USB_IDS.contains(&(vid, pid))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// What the OS reported about a port before any guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    Usb,
    Bluetooth,
    Unknown,
}

/// Decides whether a port is USB, Bluetooth or something else. The serialport
/// crate recognises Bluetooth ports on macOS and Linux, but not on Windows, so
/// names and kernel device names fill the gaps:
/// - macOS: paired SPP devices appear as `/dev/cu.<device name>`;
///   `cu.Bluetooth-Incoming-Port` is the Mac's own server port, not a tracker.
/// - Windows: "Standard Serial over Bluetooth link" ports map to `\Device\BthModemN`
///   in the SERIALCOMM registry key (`driver`).
/// - Linux: bound RFCOMM devices are `/dev/rfcommN`.
pub fn classify(os: Os, name: &str, reported: Reported, driver: Option<&str>) -> PortKind {
    let lower = name.to_ascii_lowercase();
    if os == Os::MacOs && lower.ends_with("bluetooth-incoming-port") {
        return PortKind::Other;
    }
    match reported {
        Reported::Usb => return PortKind::Usb,
        Reported::Bluetooth => return PortKind::Bluetooth,
        Reported::Unknown => {}
    }
    match os {
        Os::MacOs if lower.contains("usbmodem") || lower.contains("usbserial") => PortKind::Usb,
        Os::MacOs if lower.contains("bluetooth") || lower.contains("spp") => PortKind::Bluetooth,
        // Paired SPP devices appear as /dev/cu.<device name>, e.g. /dev/cu.FMB920_1234567.
        Os::MacOs if crate::bt_classic::model_hint(mac_device_name(name)).is_some() => PortKind::Bluetooth,
        Os::Windows if driver.is_some_and(|d| d.to_ascii_lowercase().contains("bthmodem")) => {
            PortKind::Bluetooth
        }
        Os::Linux if lower.starts_with("/dev/rfcomm") => PortKind::Bluetooth,
        Os::Linux if lower.starts_with("/dev/ttyacm") || lower.starts_with("/dev/ttyusb") => PortKind::Usb,
        _ => PortKind::Other,
    }
}

/// `/dev/cu.FMB920_1234567` -> `FMB920_1234567`.
pub fn mac_device_name(port: &str) -> &str {
    port.strip_prefix("/dev/cu.").or_else(|| port.strip_prefix("/dev/tty.")).unwrap_or(port)
}

/// Parses `reg query HKLM\HARDWARE\DEVICEMAP\SERIALCOMM` output into port -> kernel device,
/// e.g. `COM5 -> \Device\BthModem0`.
#[cfg_attr(not(windows), allow(dead_code))] // only the Windows lister calls it; tests run everywhere
pub fn parse_serialcomm(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = fields.next()?;
            (fields.next()? == "REG_SZ").then_some(())?;
            Some((fields.next()?.to_string(), device.to_string()))
        })
        .collect()
}

#[cfg(windows)]
fn windows_port_devices() -> HashMap<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg")
        .args(["query", r"HKLM\HARDWARE\DEVICEMAP\SERIALCOMM"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|out| parse_serialcomm(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn windows_port_devices() -> HashMap<String, String> {
    HashMap::new()
}

pub fn list() -> Result<Vec<PortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    let os = Os::current();
    let devices = if os == Os::Windows { windows_port_devices() } else { HashMap::new() };
    let mut list: Vec<PortInfo> = ports
        .into_iter()
        // macOS lists every device twice; the cu.* node is the one to open.
        .filter(|port| !(os == Os::MacOs && port.port_name.starts_with("/dev/tty.")))
        .map(|port| {
            let driver = devices.get(&port.port_name).map(String::as_str);
            match port.port_type {
                SerialPortType::UsbPort(usb) => PortInfo {
                    kind: classify(os, &port.port_name, Reported::Usb, driver),
                    likely_tracker: is_tracker_usb_id(usb.vid, usb.pid),
                    name: port.port_name,
                    vid: Some(usb.vid),
                    pid: Some(usb.pid),
                    manufacturer: usb.manufacturer,
                    product: usb.product,
                    serial_number: usb.serial_number,
                },
                other => {
                    let reported = match other {
                        SerialPortType::BluetoothPort => Reported::Bluetooth,
                        _ => Reported::Unknown,
                    };
                    let kind = classify(os, &port.port_name, reported, driver);
                    // On macOS the port is named after the paired device; show that name.
                    let product = (os == Os::MacOs && kind == PortKind::Bluetooth).then(|| mac_device_name(&port.port_name).to_string());
                    PortInfo {
                        kind,
                        // 01-transport v2 §2.2 applies no name filter; its Class-of-Device
                        // filter needs a native inquiry, which a serial port can't show.
                        likely_tracker: false,
                        name: port.port_name,
                        vid: None,
                        pid: None,
                        manufacturer: None,
                        product,
                        serial_number: None,
                    }
                }
            }
        })
        .collect();
    list.sort_by_key(|port| !port.likely_tracker);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_ports() {
        let mac = |name, reported| classify(Os::MacOs, name, reported, None);
        assert_eq!(mac("/dev/cu.usbmodem14101", Reported::Usb), PortKind::Usb);
        assert_eq!(mac("/dev/cu.usbmodem14101", Reported::Unknown), PortKind::Usb);
        assert_eq!(mac("/dev/cu.FMB920", Reported::Bluetooth), PortKind::Bluetooth);
        assert_eq!(mac("/dev/cu.Tracker-SPP", Reported::Unknown), PortKind::Bluetooth);
        assert_eq!(mac("/dev/cu.FMB920_1234567", Reported::Unknown), PortKind::Bluetooth);
        assert_eq!(mac_device_name("/dev/cu.FMB920_1234567"), "FMB920_1234567");
        assert_eq!(mac("/dev/cu.Bluetooth-Incoming-Port", Reported::Bluetooth), PortKind::Other);
        assert_eq!(mac("/dev/cu.debug-console", Reported::Unknown), PortKind::Other);
    }

    #[test]
    fn windows_ports() {
        let win = |name, reported, driver| classify(Os::Windows, name, reported, driver);
        assert_eq!(win("COM3", Reported::Usb, Some(r"\Device\USBSER000")), PortKind::Usb);
        assert_eq!(win("COM5", Reported::Unknown, Some(r"\Device\BthModem0")), PortKind::Bluetooth);
        assert_eq!(win("COM1", Reported::Unknown, Some(r"\Device\Serial0")), PortKind::Other);
        assert_eq!(win("COM7", Reported::Unknown, None), PortKind::Other);
    }

    #[test]
    fn linux_ports() {
        let linux = |name, reported| classify(Os::Linux, name, reported, None);
        assert_eq!(linux("/dev/ttyACM0", Reported::Usb), PortKind::Usb);
        assert_eq!(linux("/dev/ttyACM0", Reported::Unknown), PortKind::Usb);
        assert_eq!(linux("/dev/rfcomm0", Reported::Bluetooth), PortKind::Bluetooth);
        assert_eq!(linux("/dev/rfcomm1", Reported::Unknown), PortKind::Bluetooth);
        assert_eq!(linux("/dev/ttyS0", Reported::Unknown), PortKind::Other);
    }

    #[test]
    fn parses_windows_serialcomm_listing() {
        let output = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DEVICEMAP\\SERIALCOMM\r\n    \\Device\\BthModem0    REG_SZ    COM5\r\n    \\Device\\USBSER000    REG_SZ    COM3\r\n\r\n";
        let map = parse_serialcomm(output);
        assert_eq!(map.len(), 2);
        assert_eq!(map["COM5"], r"\Device\BthModem0");
        assert_eq!(map["COM3"], r"\Device\USBSER000");
    }

    #[test]
    fn tracker_usb_ids_from_spec() {
        assert!(is_tracker_usb_id(0x0483, 0x5740));
        assert!(is_tracker_usb_id(0x1D12, 0x0200));
        assert!(!is_tracker_usb_id(0x0483, 0x0001));
    }
}
