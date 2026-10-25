//! In-app pairing on macOS with `IOBluetoothDevicePair` (the reference tool never pairs;
//! this replaces a trip to System Settings). The tracker's PIN is its parameter 802,
//! default 5555 (01-transport §2.3). Legacy PIN and SSP numeric-comparison requests are
//! both answered. Runs on the main thread (sync Tauri commands); the delegate callbacks
//! arrive on the main run loop.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, Sel};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass};
use serde::Serialize;
use std::cell::RefCell;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairStatus {
    /// "idle" | "pairing" | "paired" | "failed"
    pub state: &'static str,
    pub address: Option<String>,
    pub message: Option<String>,
}

impl Default for PairStatus {
    fn default() -> Self {
        Self { state: "idle", address: None, message: None }
    }
}

struct Ivars {
    status: Arc<Mutex<PairStatus>>,
    pin: Vec<u8>,
}

extern "C" {
    fn objc_msgSend();
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FmConfigPairDelegate"]
    #[ivars = Ivars]
    struct PairDelegate;

    unsafe impl NSObjectProtocol for PairDelegate {}

    impl PairDelegate {
        #[unsafe(method(devicePairingPINCodeRequest:))]
        fn pin_request(&self, sender: *mut AnyObject) {
            let mut code = [0u8; 16];
            let pin = &self.ivars().pin;
            log::debug!("pairing: PIN requested by the device, replying ({} digits)", pin.len());
            code[..pin.len()].copy_from_slice(pin);
            // SAFETY: -[IOBluetoothDevicePair replyPINCode:(ByteCount)size PINCode:(BluetoothPINCode*)pin];
            // called through objc_msgSend with the exact C signature (struct pointer).
            unsafe {
                let f: unsafe extern "C" fn(*mut AnyObject, Sel, usize, *mut u8) =
                    std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
                f(sender, sel!(replyPINCode:PINCode:), pin.len(), code.as_mut_ptr());
            }
        }

        #[unsafe(method(devicePairingUserConfirmationRequest:numericValue:))]
        fn confirm_request(&self, sender: *mut AnyObject, _value: u32) {
            log::debug!("pairing: numeric comparison requested, confirming");
            // SAFETY: documented reply for SSP numeric comparison.
            let _: () = unsafe { msg_send![sender, replyUserConfirmation: Bool::YES] };
        }

        #[unsafe(method(devicePairingUserPasskeyNotification:passkey:))]
        fn passkey(&self, _sender: *mut AnyObject, passkey: u32) {
            log::debug!("pairing: passkey notification shown to the user");
            self.ivars().status.lock().unwrap().message = Some(format!("Enter passkey {passkey:06} on the device"));
        }

        #[unsafe(method(devicePairingFinished:error:))]
        fn finished(&self, _sender: *mut AnyObject, error: i32) {
            log::info!("pairing: finished, IOReturn 0x{error:08x}");
            let mut s = self.ivars().status.lock().unwrap();
            if error == 0 {
                s.state = "paired";
                s.message = Some("Paired".into());
            } else {
                s.state = "failed";
                s.message = Some(format!(
                    "Pairing failed (IOReturn 0x{error:08x}). Check the PIN (default 5555) and that the tracker's Bluetooth is on and visible."
                ));
            }
        }
    }
);

thread_local! {
    static ACTIVE: RefCell<Option<(Retained<AnyObject>, Retained<PairDelegate>)>> = const { RefCell::new(None) };
}

fn status_cell() -> &'static Arc<Mutex<PairStatus>> {
    static S: std::sync::OnceLock<Arc<Mutex<PairStatus>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn status() -> PairStatus {
    status_cell().lock().unwrap().clone()
}

/// Starts pairing; poll [`status`] for the result. Main thread only.
pub fn start(address: &str, pin: &str) -> Result<PairStatus, String> {
    if pin.is_empty() || pin.len() > 16 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err("The PIN must be 1–16 digits (the tracker default is 5555)".into());
    }
    let cls = |n: &std::ffi::CStr| AnyClass::get(n).ok_or_else(|| "Bluetooth is not available on this Mac".to_string());
    let addr = CString::new(address).map_err(|e| e.to_string())?;
    let st = status_cell().clone();
    *st.lock().unwrap() = PairStatus { state: "pairing", address: Some(address.into()), message: Some("Pairing…".into()) };
    // SAFETY: documented IOBluetooth class/instance methods; nil results are handled.
    let r: i32 = unsafe {
        let ns: Option<Retained<AnyObject>> = msg_send![cls(c"NSString")?, stringWithUTF8String: addr.as_ptr()];
        let ns = ns.ok_or("bad address")?;
        let dev: Option<Retained<AnyObject>> = msg_send![cls(c"IOBluetoothDevice")?, deviceWithAddressString: &*ns];
        let dev = dev.ok_or_else(|| format!("No Bluetooth device at {address}"))?;
        let pair: Option<Retained<AnyObject>> = msg_send![cls(c"IOBluetoothDevicePair")?, pairWithDevice: &*dev];
        let pair = pair.ok_or("Could not start pairing")?;
        let delegate = PairDelegate::alloc().set_ivars(Ivars { status: st.clone(), pin: pin.as_bytes().to_vec() });
        let delegate: Retained<PairDelegate> = msg_send![super(delegate), init];
        let _: () = msg_send![&*pair, setDelegate: &*delegate];
        let r: i32 = msg_send![&*pair, start];
        ACTIVE.with(|a| *a.borrow_mut() = Some((pair, delegate)));
        r
    };
    log::info!("pairing: started for {address}, IOReturn 0x{r:08x}");
    if r != 0 {
        let mut s = st.lock().unwrap();
        s.state = "failed";
        s.message = Some(format!("Pairing could not start (IOReturn 0x{r:08x}). Is Bluetooth on?"));
    }
    Ok(status())
}
