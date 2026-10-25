//! macOS RFCOMM transport to the tracker's SPP service (01-transport §2.1).
//!
//! Modern macOS no longer creates `/dev/cu.*` ports for SPP devices, so we talk RFCOMM
//! directly through IOBluetooth, as the reference tool does with a socket.
//!
//! Threading: IOBluetooth delivers channel delegate callbacks on the run loop of the
//! thread that opened the channel. We open, write and close on the **main thread**
//! (Tauri's main run loop, via `run_on_main_thread`), so the delegate receives data
//! there and appends it to a shared buffer. `read` runs on the session's worker thread
//! and waits on a condvar; it never touches IOBluetooth. Never call these from the main
//! thread while it is blocked waiting on the worker (we detect the main thread and run
//! inline instead of dispatching).

use crate::transport::Transport;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use std::collections::VecDeque;
use std::ffi::{c_void, CString};
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// SPP service class (00001101-0000-1000-8000-00805F9B34FB) as a 16-bit UUID.
const SPP_UUID16: u16 = 0x1101;
const IO_SUCCESS: i32 = 0;
const IO_BUSY: i32 = 0xE000_02D5_u32 as i32; // kIOReturnBusy
const IO_EXCLUSIVE: i32 = 0xE000_02C5_u32 as i32; // kIOReturnExclusiveAccess
const IO_BAD_ARG: i32 = 0xE000_02C2_u32 as i32; // kIOReturnBadArgument
/// How long to wait for an SDP query when the device has no cached SPP record.
const SDP_WAIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Rx {
    state: Mutex<(VecDeque<u8>, bool)>, // (bytes, closed)
    cv: Condvar,
}

struct Ivars {
    rx: Arc<Rx>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FmConfigRfcommDelegate"]
    #[ivars = Ivars]
    struct RfDelegate;

    unsafe impl NSObjectProtocol for RfDelegate {}

    impl RfDelegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn on_data(&self, _ch: *mut AnyObject, data: *mut c_void, len: usize) {
            if data.is_null() || len == 0 {
                return;
            }
            // SAFETY: IOBluetooth passes `len` valid bytes at `data` for this call.
            let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
            let rx = &self.ivars().rx;
            log::trace!("rfcomm: delegate data, {len} bytes");
            rx.state.lock().unwrap().0.extend(bytes);
            rx.cv.notify_all();
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn on_closed(&self, _ch: *mut AnyObject) {
            let rx = &self.ivars().rx;
            log::warn!("rfcomm: delegate reported the channel closed");
            rx.state.lock().unwrap().1 = true;
            rx.cv.notify_all();
        }
    }
);

impl RfDelegate {
    fn new(rx: Arc<Rx>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { rx });
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Objective-C objects that are only ever used (and dropped) on the main thread.
struct MainOnly<T>(T);
// SAFETY: the wrapped objects are only touched inside `on_main` closures.
unsafe impl<T> Send for MainOnly<T> {}

struct Open {
    channel: Retained<AnyObject>,
    _delegate: Retained<RfDelegate>,
    device: Retained<AnyObject>,
    mtu: u16,
}

fn is_main() -> bool {
    extern "C" {
        fn pthread_main_np() -> i32;
    }
    // SAFETY: no preconditions.
    unsafe { pthread_main_np() != 0 }
}

fn on_main<R: Send + 'static>(app: &AppHandle, f: impl FnOnce() -> R + Send + 'static) -> Result<R, String> {
    if is_main() {
        return Ok(f());
    }
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "Bluetooth did not respond".to_string())
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
    AnyClass::get(name).ok_or_else(|| "Bluetooth is not available on this Mac".to_string())
}

/// Main thread only.
fn device(address: &str) -> Result<Retained<AnyObject>, String> {
    let s = CString::new(address).map_err(|e| e.to_string())?;
    // SAFETY: documented class methods; nil results are handled.
    unsafe {
        let ns: Option<Retained<AnyObject>> = msg_send![class(c"NSString")?, stringWithUTF8String: s.as_ptr()];
        let ns = ns.ok_or("bad address")?;
        let d: Option<Retained<AnyObject>> = msg_send![class(c"IOBluetoothDevice")?, deviceWithAddressString: &*ns];
        d.ok_or_else(|| format!("No Bluetooth device at {address}"))
    }
}

/// Main thread only. The SPP RFCOMM channel from the device's cached SDP record.
fn spp_channel(dev: &AnyObject) -> Result<Option<u8>, String> {
    // SAFETY: documented methods; out-parameter is a valid u8.
    unsafe {
        let uuid: Option<Retained<AnyObject>> = msg_send![class(c"IOBluetoothSDPUUID")?, uuid16: SPP_UUID16];
        let uuid = uuid.ok_or("SDP UUID")?;
        let rec: Option<Retained<AnyObject>> = msg_send![dev, getServiceRecordForUUID: &*uuid];
        let Some(rec) = rec else {
            log::debug!("rfcomm: no cached SPP service record");
            return Ok(None);
        };
        let mut ch: u8 = 0;
        let r: i32 = msg_send![&*rec, getRFCOMMChannelID: &mut ch as *mut u8];
        log::debug!("rfcomm: SPP service record found, getRFCOMMChannelID returned 0x{r:08x}, channel {ch}");
        Ok((r == IO_SUCCESS).then_some(ch))
    }
}

fn open_once(app: &AppHandle, address: &str, rx: &Arc<Rx>) -> Result<MainOnly<Open>, (String, bool)> {
    // 1. Resolve the SPP channel through SDP (cached record first, else query).
    let addr = address.to_string();
    let first = on_main(app, move || -> Result<Option<u8>, String> {
        let d = device(&addr)?;
        if let Some(ch) = spp_channel(&d)? {
            return Ok(Some(ch));
        }
        // SAFETY: documented; nil target means "notify nobody", results land in the device's cache.
        let r: i32 = unsafe { msg_send![&*d, performSDPQuery: std::ptr::null::<AnyObject>()] };
        log::info!("rfcomm: SDP query started, IOReturn 0x{r:08x}");
        if r != IO_SUCCESS {
            return Err(format!("SDP query failed (IOReturn 0x{r:08x})"));
        }
        Ok(None)
    })
    .and_then(|r| r)
    .map_err(|e| (e, false))?;
    log::info!("rfcomm: cached SPP channel for {address}: {first:?}");
    let channel_id = match first {
        Some(ch) => ch,
        None => {
            let until = Instant::now() + SDP_WAIT;
            loop {
                std::thread::sleep(Duration::from_millis(300));
                let addr = address.to_string();
                if let Some(ch) = on_main(app, move || spp_channel(&*device(&addr)?)).and_then(|r| r).map_err(|e| (e, false))? {
                    break ch;
                }
                if Instant::now() > until {
                    log::error!("rfcomm: SDP wait of {} s ended without an SPP service", SDP_WAIT.as_secs());
                    return Err(("The device did not report a Serial Port (SPP) service. Is its Bluetooth on and in range?".into(), false));
                }
            }
        }
    };

    // 2. Open the RFCOMM channel with our delegate.
    let addr = address.to_string();
    let rx = rx.clone();
    on_main(app, move || -> Result<MainOnly<Open>, (String, bool)> {
        let d = device(&addr).map_err(|e| (e, false))?;
        let delegate = RfDelegate::new(rx);
        let mut chan: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: documented; `chan` receives the channel object on success.
        let r: i32 = unsafe {
            msg_send![&*d, openRFCOMMChannelSync: &mut chan as *mut *mut AnyObject, withChannelID: channel_id, delegate: &*delegate]
        };
        // SAFETY: on success `chan` is a valid IOBluetoothRFCOMMChannel; we take our own reference.
        log::info!("rfcomm: openRFCOMMChannelSync on channel {channel_id} returned 0x{r:08x}");
        let channel = (r == IO_SUCCESS).then(|| unsafe { Retained::retain(chan) }).flatten();
        let Some(channel) = channel else {
            let busy = r == IO_BUSY || r == IO_EXCLUSIVE || r == IO_BAD_ARG;
            return Err((format!("Could not open the Bluetooth serial channel (IOReturn 0x{r:08x}). Pair the tracker first (default PIN 5555) and make sure no other app is connected to it."), busy));
        };
        // SAFETY: documented accessor.
        let mtu: u16 = unsafe { msg_send![&*channel, getMTU] };
        log::info!("rfcomm: channel open, MTU {mtu}");
        Ok(MainOnly(Open { channel, _delegate: delegate, device: d, mtu: mtu.max(1) }))
    })
    .map_err(|e| (e, false))?
}

pub struct RfcommTransport {
    app: AppHandle,
    address: String,
    rx: Arc<Rx>,
    open: Option<MainOnly<Open>>,
    reconnected: bool,
}

impl RfcommTransport {
    /// 01-transport §2.1: wait 100 ms, connect, retry "in use"/"invalid argument" up to 3 attempts.
    pub fn open(app: AppHandle, address: &str) -> Result<Self, String> {
        log::info!("rfcomm: opening {address}");
        let rx = Arc::new(Rx::default());
        let open = Self::connect(&app, address, &rx)?;
        Ok(Self { app, address: address.to_string(), rx, open: Some(open), reconnected: false })
    }

    fn connect(app: &AppHandle, address: &str, rx: &Arc<Rx>) -> Result<MainOnly<Open>, String> {
        std::thread::sleep(Duration::from_millis(100));
        let mut last = String::new();
        for attempt in 1..=3 {
            match open_once(app, address, rx) {
                Ok(o) => {
                    *rx.state.lock().unwrap() = (VecDeque::new(), false);
                    return Ok(o);
                }
                Err((m, busy)) => {
                    log::warn!("rfcomm: open attempt {attempt} of 3 failed (busy {busy}): {m}");
                    last = m;
                    if !busy || attempt == 3 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        Err(last)
    }

    fn close(&mut self) {
        if let Some(o) = self.open.take() {
            log::info!("rfcomm: closing channel to {}", self.address);
            let _ = on_main(&self.app, move || {
                let o = o;
                // SAFETY: documented methods on objects owned by this transport.
                unsafe {
                    let _: i32 = msg_send![&*o.0.channel, closeChannel];
                    let _: i32 = msg_send![&*o.0.device, closeConnection];
                }
                drop(o);
            });
        }
    }
}

impl Drop for RfcommTransport {
    fn drop(&mut self) {
        self.close();
    }
}

impl Transport for RfcommTransport {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(o) = self.open.as_ref() else { return Err(io::Error::new(io::ErrorKind::NotConnected, "Bluetooth link closed")) };
        let mtu = o.0.mtu as usize;
        let chan = MainOnly(o.0.channel.clone());
        let data = bytes.to_vec();
        let r = on_main(&self.app, move || {
            let chan = chan;
            for chunk in data.chunks(mtu) {
                // SAFETY: documented; the buffer outlives the synchronous call.
                let r: i32 = unsafe { msg_send![&*chan.0, writeSync: chunk.as_ptr() as *mut c_void, length: chunk.len() as u16] };
                if r != IO_SUCCESS {
                    return r;
                }
            }
            IO_SUCCESS
        })
        .map_err(io::Error::other)?;
        if r == IO_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, format!("Bluetooth write failed (IOReturn 0x{r:08x})")))
        }
    }

    fn read(&mut self, buf: &mut [u8], deadline: Instant) -> io::Result<usize> {
        loop {
            let mut g = self.rx.state.lock().unwrap();
            if !g.0.is_empty() {
                let n = buf.len().min(g.0.len());
                for (i, b) in g.0.drain(..n).enumerate() {
                    buf[i] = b;
                }
                return Ok(n);
            }
            if g.1 {
                drop(g);
                // §2.1: one reconnect attempt to the same address when the link dropped.
                if self.reconnected {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Bluetooth link dropped"));
                }
                self.reconnected = true;
                log::warn!("rfcomm: link dropped, reconnecting once");
                self.close();
                let addr = self.address.clone();
                self.open = Some(Self::connect(&self.app, &addr, &self.rx).map_err(io::Error::other)?);
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(0);
            }
            let _ = self.rx.cv.wait_timeout(g, deadline - now).unwrap();
        }
    }

    fn clear_input(&mut self) -> io::Result<()> {
        self.rx.state.lock().unwrap().0.clear();
        Ok(())
    }
}
