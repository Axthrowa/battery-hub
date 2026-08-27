//! Shared hidapi context + editable VID/PID table.
//!
//! IDs below were filled from this machine's diagnostic scan (2026-08-25):
//!   Logitech LIGHTSPEED receiver       VID=0x046D PID=0xC54D
//!   Logitech PRO X2 SUPERSTRIKE        VID=0x046D PID=0xC0A8  (HID++ battery)
//!   AJAZZ 2.4G 8K                       VID=0x3151 PID=0x5007
//!   soundcore Select 4 Go              (Bluetooth name match)

use hidapi::{BusType, DeviceInfo, HidApi, HidResult};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

pub const AJAZZ_VIDS: &[u16] = &[
    0x3151, // AJAZZ 2.4G 8K (this machine)
    // 0x3554 and 0x258A were here until the Aula reader was written. Both are
    // Compx, and this reader probes by writing 0xF7 frames to every candidate
    // it can reach: aimed at an Aula receiver those frames leave its own
    // request/response channel unable to answer, so the keyboard reads as
    // silent while a mouse two entries up is reporting fine.
    0x1A2C, 0x093A, 0x18F8,
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

/// Strings a receiver puts in its USB descriptor. A 2.4 GHz dongle is an
/// ordinary USB device to Windows, so the bus alone cannot separate it from a
/// cable — only the receiver's own name can.
const RECEIVER_HINTS: &[&str] = &["2.4g", "2.4 g", "receiver", "dongle"];

/// What to show as the link for a device the generic readers picked up.
///
/// Bluetooth is decided by the bus (Windows reports it through the compatible
/// IDs); everything else is USB-rooted, where a receiver is recognised by name
/// and anything unrecognised keeps the old neutral label.
pub fn transport_label(info: &DeviceInfo) -> &'static str {
    if matches!(info.bus_type(), BusType::Bluetooth) {
        return "Bluetooth";
    }
    let name = format!(
        "{} {}",
        info.manufacturer_string().unwrap_or_default(),
        info.product_string().unwrap_or_default()
    )
    .to_ascii_lowercase();
    if RECEIVER_HINTS.iter().any(|hint| name.contains(hint)) {
        "2.4 GHz"
    } else {
        "HID"
    }
}

/// Vendors a dedicated reader already speaks for.
///
/// Their hardware needs no teaching, and offering it anyway is actively
/// harmful: the scan can only propose bytes that hold still, and on a keyboard
/// whose charge lives behind a request/response frame the steady bytes are
/// firmware constants. Someone picks the one nearest the real percentage and
/// the app reports that number for good.
pub fn covered_by_a_reader(vendor_id: u16) -> bool {
    vendor_id == RAZER_VID
        || vendor_id == LOGITECH_VID
        || AJAZZ_VIDS.contains(&vendor_id)
        || super::aula::AULA_VIDS.contains(&vendor_id)
}

fn hid_context() -> &'static Mutex<Option<HidApi>> {
    static CONTEXT: OnceLock<Mutex<Option<HidApi>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Enumerating every HID interface means opening every one of them, and the
/// readers run eight deep behind this lock: doing it per reader put the same
/// hardware through it eight times a poll, which cost seconds and left 2.4 GHz
/// receivers too busy to answer their own vendor frames. One sweep is enough
/// for a whole round of readers.
const ENUMERATION_TTL: Duration = Duration::from_secs(2);

fn last_enumeration() -> &'static Mutex<Option<Instant>> {
    static AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    AT.get_or_init(|| Mutex::new(None))
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

    let stale = last_enumeration()
        .lock()
        .map(|at| at.is_none_or(|at| at.elapsed() >= ENUMERATION_TTL))
        .unwrap_or(true);
    if stale {
        if let Err(e) = refresh_device_list(api) {
            return Err(format!("HID enumeration failed: {e}"));
        }
        if let Ok(mut at) = last_enumeration().lock() {
            *at = Some(Instant::now());
        }
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
