//! Shared hidapi context + editable VID/PID table.
//!
//! IDs below were filled from this machine's diagnostic scan (2026-08-25):
//!   Logitech LIGHTSPEED receiver       VID=0x046D PID=0xC54D
//!   Logitech PRO X2 SUPERSTRIKE        VID=0x046D PID=0xC0A8  (HID++ battery)
//!   AJAZZ 2.4G 8K                       VID=0x3151 PID=0x5007
//!   soundcore Select 4 Go              (Bluetooth name match)

use hidapi::{HidApi, HidResult};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// RAZER — BlackShark V2 HyperSpeed (dongle + optional wired)
// ---------------------------------------------------------------------------
pub const RAZER_VID: u16 = 0x1532;
pub const RAZER_PID_DONGLE: u16 = 0x0565;
pub const RAZER_PID_WIRED: u16 = 0x056E;

// ---------------------------------------------------------------------------
// LOGITECH — Unifying / Lightspeed / Bolt USB receiver (HID++)
// ---------------------------------------------------------------------------
// BURAYA LOGITECH VID GİRİLECEK: 0x046D
pub const LOGITECH_VID: u16 = 0x046D;

// None = scan receivers + direct HID++ peripherals (needed for accurate %).
// Some(pid) = only that PID (debug). This PC: receiver 0xC54D + mouse 0xC0A8.
pub const LOGITECH_PID_OVERRIDE: Option<u16> = None;

#[allow(dead_code)]
pub const LOGITECH_PIDS: &[u16] = &[
    // Receivers
    0xC52B, 0xC532, 0xC539, 0xC53A, 0xC53D, 0xC53F, 0xC541, 0xC545, 0xC547, 0xC548, 0xC54D,
    // Direct / wireless-presented mice (PRO X2 SUPERSTRIKE on this PC)
    0xC0A8, 0xC077, 0xC08B, 0xC093, 0xC09D,
];

// ---------------------------------------------------------------------------
// AJAZZ — 2.4 GHz OEM receivers (Usage Page 0xFF00 / 0xFFFF / Battery 0x85)
// ---------------------------------------------------------------------------
// BURAYA AJAZZ VID GİRİLECEK: 0x3151
pub const AJAZZ_VID_PRIMARY: u16 = 0x3151;

#[allow(dead_code)]
// None = any Ajazz / OEM VID in AJAZZ_VIDS (multi-brand 2.4 GHz mice).
pub const AJAZZ_PID_OVERRIDE: Option<u16> = None;

pub const AJAZZ_VIDS: &[u16] = &[
    0x3151, // AJAZZ 2.4G 8K (this machine)
    0x3554, 0x258A, 0x1A2C, 0x093A, 0x18F8,
];

// ---------------------------------------------------------------------------
// SOUNDCORE — Bluetooth friendly-name filter
// ---------------------------------------------------------------------------
pub const SOUNDCORE_NAME_HINTS: &[&str] = &[
    "soundcore",
    "select 4 go",
    "select go",
    "anker life",
    "anker sound",
    "anker",
];

fn hid_context() -> &'static Mutex<Option<HidApi>> {
    static CONTEXT: OnceLock<Mutex<Option<HidApi>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
}

pub fn with_api<T>(f: impl FnOnce(&HidApi) -> T) -> Result<T, String> {
    let mut guard = hid_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if guard.is_none() {
        match HidApi::new() {
            Ok(api) => *guard = Some(api),
            Err(e) => return Err(format!("HID init failed: {e}")),
        }
    }
    let api = guard.as_mut().expect("initialised above");
    if let Err(e) = refresh_device_list(api) {
        return Err(format!("HID enumeration failed: {e}"));
    }
    Ok(f(api))
}

fn refresh_device_list(api: &mut HidApi) -> HidResult<()> {
    // Enumerate all HID devices so any brand with a battery page / vendor
    // interface can be discovered (specialized readers still filter by VID).
    api.reset_devices()?;
    api.add_devices(0, 0)?;
    Ok(())
}
