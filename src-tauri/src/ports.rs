//! Serial ports a tracker could be connected on (USB CDC or Bluetooth SPP).

use serde::Serialize;
use serialport::SerialPortType;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub name: String,
    pub kind: &'static str,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

pub fn list() -> Result<Vec<PortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    Ok(ports
        .into_iter()
        // macOS lists every device twice; the cu.* node is the one to open.
        .filter(|port| !(cfg!(target_os = "macos") && port.port_name.starts_with("/dev/tty.")))
        .map(|port| match port.port_type {
            SerialPortType::UsbPort(usb) => PortInfo {
                name: port.port_name,
                kind: "usb",
                vid: Some(usb.vid),
                pid: Some(usb.pid),
                manufacturer: usb.manufacturer,
                product: usb.product,
                serial_number: usb.serial_number,
            },
            other => PortInfo {
                name: port.port_name,
                kind: match other {
                    SerialPortType::BluetoothPort => "bluetooth",
                    SerialPortType::PciPort => "pci",
                    _ => "unknown",
                },
                vid: None,
                pid: None,
                manufacturer: None,
                product: None,
                serial_number: None,
            },
        })
        .collect())
}
