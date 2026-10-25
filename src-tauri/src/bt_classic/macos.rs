//! macOS backend: IOBluetooth `IOBluetoothDeviceInquiry` and `IOBluetoothDevice.pairedDevices`.
//!
//! Minimal `objc2` message sends (no delegate class). The inquiry is started without a
//! delegate and its `foundDevices` list is polled. IOBluetooth delivers results on the
//! run loop of the thread that started the inquiry, so every call here must happen on
//! the main thread. The Tauri commands that drive it are sync, and Tauri runs those
//! on the main thread. The inquiry object lives in a main-thread `thread_local`.

use super::{ClassicScanner, Sighting, INQUIRY_SECS};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use std::cell::RefCell;
use std::ffi::CStr;

#[link(name = "IOBluetooth", kind = "framework")]
extern "C" {}

thread_local! {
    static INQUIRY: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// kIOReturnSuccess
const IO_SUCCESS: i32 = 0;
/// RSSI value IOBluetooth reports when it has none.
const RSSI_INVALID: i8 = 127;

#[derive(Default)]
pub struct MacScanner;

fn class(name: &CStr) -> Result<&'static AnyClass, String> {
    AnyClass::get(name).ok_or_else(|| "Bluetooth is not available on this Mac".to_string())
}

fn main_thread() -> Result<(), String> {
    // SAFETY: pthread_main_np has no preconditions.
    extern "C" {
        fn pthread_main_np() -> i32;
    }
    if unsafe { pthread_main_np() } == 0 {
        return Err("internal: Bluetooth inquiry must run on the main thread".into());
    }
    Ok(())
}

fn nsstring(s: Option<Retained<AnyObject>>) -> Option<String> {
    let s = s?;
    // SAFETY: `s` is an NSString; UTF8String returns a NUL-terminated buffer valid while `s` lives.
    let p: *const std::ffi::c_char = unsafe { msg_send![&*s, UTF8String] };
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// Reads an IOBluetoothDevice into a [`Sighting`].
fn sighting(dev: &AnyObject) -> Option<Sighting> {
    // SAFETY: `dev` is an IOBluetoothDevice; these are its documented accessors and types.
    unsafe {
        let address = nsstring(msg_send![dev, addressString])?;
        let name = nsstring(msg_send![dev, name]);
        let cod: u32 = msg_send![dev, classOfDevice];
        let rssi: i8 = msg_send![dev, RSSI];
        let paired: Bool = msg_send![dev, isPaired];
        Some(Sighting {
            address,
            name,
            class_of_device: cod,
            rssi: (rssi != RSSI_INVALID && rssi != 0).then_some(rssi as i16),
            paired: paired.as_bool(),
        })
    }
}

/// Reads every element of an NSArray of IOBluetoothDevice.
fn devices(arr: Option<Retained<AnyObject>>) -> Vec<Sighting> {
    let Some(arr) = arr else { return Vec::new() };
    // SAFETY: `arr` is an NSArray; indexes are in range.
    unsafe {
        let n: usize = msg_send![&*arr, count];
        (0..n)
            .filter_map(|i| {
                let d: Option<Retained<AnyObject>> = msg_send![&*arr, objectAtIndex: i];
                d.and_then(|d| sighting(&d))
            })
            .collect()
    }
}

impl ClassicScanner for MacScanner {
    fn start_inquiry(&mut self) -> Result<(), String> {
        main_thread()?;
        INQUIRY.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let cls = class(c"IOBluetoothDeviceInquiry")?;
                // SAFETY: class method `+inquiryWithDelegate:` accepts nil.
                let inq: Option<Retained<AnyObject>> = unsafe { msg_send![cls, inquiryWithDelegate: std::ptr::null::<AnyObject>()] };
                let inq = inq.ok_or("Could not create a Bluetooth inquiry")?;
                // SAFETY: documented setters; inquiryLength is uint8_t seconds.
                unsafe {
                    let _: () = msg_send![&*inq, setInquiryLength: INQUIRY_SECS as u8];
                    let _: () = msg_send![&*inq, setUpdateNewDeviceNames: Bool::YES];
                }
                *slot = Some(inq);
            }
            let inq = slot.as_ref().unwrap();
            // SAFETY: documented methods; stop on an idle inquiry is harmless.
            let r: i32 = unsafe {
                let _: i32 = msg_send![&**inq, stop];
                let _: () = msg_send![&**inq, clearFoundDevices];
                msg_send![&**inq, start]
            };
            log::debug!("bluetooth classic: IOBluetoothDeviceInquiry start returned 0x{r:08x}");
            if r == IO_SUCCESS {
                Ok(())
            } else {
                Err(format!("Bluetooth inquiry could not start (IOReturn 0x{r:08x}). Is Bluetooth on, and is the app allowed to use it?"))
            }
        })
    }

    fn stop_inquiry(&mut self) {
        if main_thread().is_err() {
            return;
        }
        INQUIRY.with(|cell| {
            if let Some(inq) = cell.borrow().as_ref() {
                // SAFETY: documented method.
                let _: i32 = unsafe { msg_send![&**inq, stop] };
            }
        });
    }

    fn found(&mut self) -> Result<Vec<Sighting>, String> {
        main_thread()?;
        Ok(INQUIRY.with(|cell| match cell.borrow().as_ref() {
            // SAFETY: documented accessor returning an NSArray (or nil).
            Some(inq) => devices(unsafe { msg_send![&**inq, foundDevices] }),
            None => Vec::new(),
        }))
    }

    fn paired(&mut self) -> Result<Vec<Sighting>, String> {
        main_thread()?;
        let cls = class(c"IOBluetoothDevice")?;
        // SAFETY: class method returning an NSArray (or nil).
        let mut v = devices(unsafe { msg_send![cls, pairedDevices] });
        v.iter_mut().for_each(|d| d.paired = true);
        Ok(v)
    }
}
