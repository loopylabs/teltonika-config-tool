//! Linux backend: BlueZ through `bluer`. UNTESTED: written on macOS, never compiled here.
//!
//! BlueZ discovery is continuous, so one background task runs it (BR/EDR transport only)
//! and re-reads the adapter's devices every 2 s. A device with an RSSI value was heard
//! during this discovery session; those become sightings. The paired list is cached by
//! the same task.

use super::{ClassicScanner, Sighting};
use bluer::{Adapter, AdapterEvent, DiscoveryFilter, DiscoveryTransport};
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;

#[derive(Default)]
struct Shared {
    found: Vec<Sighting>,
    paired: Vec<Sighting>,
    error: Option<String>,
}

#[derive(Default)]
pub struct BluezScanner {
    shared: Arc<Mutex<Shared>>,
    task: Option<JoinHandle<()>>,
}

async fn read_device(adapter: &Adapter, addr: bluer::Address) -> bluer::Result<Sighting> {
    let d = adapter.device(addr)?;
    Ok(Sighting {
        address: addr.to_string(),
        name: d.name().await?,
        class_of_device: d.class().await?.unwrap_or(0),
        rssi: d.rssi().await?,
        paired: d.is_paired().await?,
    })
}

async fn run(shared: Arc<Mutex<Shared>>) -> bluer::Result<()> {
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    if !adapter.is_powered().await? {
        return Err(bluer::Error { kind: bluer::ErrorKind::NotReady, message: "Bluetooth is off".into() });
    }
    adapter
        .set_discovery_filter(DiscoveryFilter { transport: DiscoveryTransport::BrEdr, ..Default::default() })
        .await?;
    let mut events = adapter.discover_devices().await?;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            ev = events.next() => match ev {
                Some(AdapterEvent::DeviceAdded(_)) | Some(_) => {}
                None => return Ok(()),
            },
            _ = tick.tick() => {
                let mut found = Vec::new();
                let mut paired = Vec::new();
                for addr in adapter.device_addresses().await? {
                    if let Ok(s) = read_device(&adapter, addr).await {
                        if s.paired {
                            paired.push(s.clone());
                        }
                        if s.rssi.is_some() {
                            found.push(s);
                        }
                    }
                }
                let mut g = shared.lock().unwrap();
                g.found.extend(found);
                g.paired = paired;
            }
        }
    }
}

impl ClassicScanner for BluezScanner {
    fn start_inquiry(&mut self) -> Result<(), String> {
        if let Some(e) = self.shared.lock().unwrap().error.take() {
            self.task = None;
            return Err(e);
        }
        if self.task.is_none() {
            let shared = self.shared.clone();
            self.task = Some(tauri::async_runtime::spawn(async move {
                if let Err(e) = run(shared.clone()).await {
                    shared.lock().unwrap().error = Some(format!("Bluetooth discovery failed: {e}"));
                }
            }));
        }
        Ok(())
    }

    fn stop_inquiry(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort(); // dropping the discovery stream ends discovery
        }
    }

    fn found(&mut self) -> Result<Vec<Sighting>, String> {
        let mut g = self.shared.lock().unwrap();
        if let Some(e) = g.error.clone() {
            return Err(e);
        }
        Ok(std::mem::take(&mut g.found))
    }

    fn paired(&mut self) -> Result<Vec<Sighting>, String> {
        Ok(self.shared.lock().unwrap().paired.clone())
    }
}
