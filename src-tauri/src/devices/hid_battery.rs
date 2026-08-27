//! Generic HID battery — vendor independent, driven by the report descriptor.
//!
//! Every HID interface is asked for its report descriptor; if it declares a
//! state-of-charge field (Battery System page `0x85`, or Generic Device
//! Controls `Battery Strength`) the exact bits are read. Devices whose
//! descriptor hides the field fall back to a byte scan, but only inside
//! collections that are supposed to carry a battery.
//!
//! Note: hidapi's `windows-native` backend compiles no `get_input_report`
//! (it is gated on the C library / Linux), so fields that live in an Input
//! report are captured from the interrupt endpoint and matched by report ID.

use super::hid;
use super::hid_descriptor::{self as desc, BatteryField, BatteryLayout, ReportKind};
use super::{Brand, DeviceReading};
use hidapi::HidDevice;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// HID_MAX_DESCRIPTOR_SIZE.
const DESCRIPTOR_MAX: usize = 4096;
/// Matches the report size the specialized readers use on this hardware.
const REPORT_MAX: usize = 64;
const READ_TIMEOUT_MS: i32 = 40;
/// Upper bound on waiting for a battery Input report to arrive.
const INPUT_BUDGET_MS: u64 = 120;
/// Report IDs commonly used by battery collections, 0x00 last (unnumbered).
const HEURISTIC_REPORT_IDS: [u8; 9] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x00];

/// Report descriptors are static per interface, so each path is parsed once:
/// after the first poll only devices that actually declare a battery field
/// (or sit on a battery page) are opened again.
fn layout_cache() -> &'static Mutex<HashMap<String, Option<BatteryLayout>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<BatteryLayout>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn layout_of(dev: &HidDevice) -> Option<BatteryLayout> {
    let mut raw = [0u8; DESCRIPTOR_MAX];
    let read = dev.get_report_descriptor(&mut raw).ok()?;
    let end = read.min(DESCRIPTOR_MAX);
    (end > 0).then(|| desc::parse(&raw[..end]))
}

/// Feature reports come back with the report ID as byte 0 — even for
/// unnumbered devices, where it is `0x00`. The payload starts at byte 1.
fn feature_payload(dev: &HidDevice, report_id: u8) -> Option<Vec<u8>> {
    let mut buf = [0u8; REPORT_MAX];
    buf[0] = report_id;
    let read = dev.get_feature_report(&mut buf).ok()?;
    let end = read.min(REPORT_MAX);
    (end > 1).then(|| buf[1..end].to_vec())
}

/// Wait for one interrupt report belonging to `report_id`. Unlike feature
/// reports, an interrupt read carries the leading ID byte only when the
/// descriptor declares report IDs — reading it as data is the `0x02 → 2%` bug.
fn input_payload(dev: &HidDevice, report_id: u8, uses_report_ids: bool) -> Option<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_millis(INPUT_BUDGET_MS);
    loop {
        let mut buf = [0u8; REPORT_MAX];
        let read = dev.read_timeout(&mut buf, READ_TIMEOUT_MS).ok()?;
        let end = read.min(REPORT_MAX);
        if end > 0 {
            if !uses_report_ids {
                return Some(buf[..end].to_vec());
            }
            // The endpoint also carries unrelated reports (mouse moves, keys).
            if buf[0] == report_id && end > 1 {
                return Some(buf[1..end].to_vec());
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
    }
}

fn payload_for(dev: &HidDevice, field: &BatteryField, uses_report_ids: bool) -> Option<Vec<u8>> {
    match field.kind {
        ReportKind::Feature => feature_payload(dev, field.report_id),
        ReportKind::Input => input_payload(dev, field.report_id, uses_report_ids),
    }
}

/// Exact read: the descriptor told us the report, the offset and the range.
fn read_from_layout(
    dev: &HidDevice,
    layout: &BatteryLayout,
    uses_report_ids: bool,
) -> Option<(u8, bool)> {
    let charge = layout.charge?;
    let payload = payload_for(dev, &charge, uses_report_ids)?;
    let percent = desc::to_percent(desc::field_value(&payload, &charge)?, &charge)?;

    let charging = match layout.charging {
        // Same report as the SOC field — no second round trip.
        Some(flag) if flag.same_report(&charge) => {
            desc::field_value(&payload, &flag).unwrap_or(0) != 0
        }
        Some(flag) => {
            payload_for(dev, &flag, uses_report_ids)
                .and_then(|bytes| desc::field_value(&bytes, &flag))
                .unwrap_or(0)
                != 0
        }
        None => false,
    };

    Some((percent, charging))
}

/// Fallback for descriptors that declare no battery usage: probe the usual
/// report IDs and take the first byte that reads like a percentage.
fn read_heuristic(dev: &HidDevice, uses_report_ids: bool) -> Option<(u8, bool)> {
    for report_id in HEURISTIC_REPORT_IDS {
        let mut buf = [0u8; REPORT_MAX];
        buf[0] = report_id;
        if let Ok(read) = dev.get_feature_report(&mut buf) {
            if let Some(percent) = desc::plausible_percent(&buf[..read.min(REPORT_MAX)], true) {
                return Some((percent, false));
            }
        }
    }

    // Some devices only publish battery on the interrupt endpoint.
    let mut buf = [0u8; REPORT_MAX];
    let read = dev.read_timeout(&mut buf, READ_TIMEOUT_MS).ok()?;
    desc::plausible_percent(&buf[..read.min(REPORT_MAX)], uses_report_ids)
        .map(|percent| (percent, false))
}

/// Collections where a byte scan is defensible rather than reckless.
fn battery_collection(usage_page: u16) -> bool {
    usage_page == desc::PAGE_BATTERY_SYSTEM || usage_page == desc::PAGE_GENERIC_DEVICE
}

/// Laptop / UPS batteries — Battery Hub is for peripherals.
fn is_system_battery(manufacturer: &str, product: &str) -> bool {
    let hay = format!("{manufacturer} {product}").to_ascii_lowercase();
    hay.contains("system")
        || hay.contains("acpi")
        || hay.contains("primary battery")
        || hay.contains("microsoft acpi")
}

/// Probe every HID interface for a standard state-of-charge field.
pub fn read_all() -> Vec<DeviceReading> {
    let result = hid::with_api(|api| {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for info in api.device_list() {
            let key = format!(
                "{:04X}:{:04X}:{}",
                info.vendor_id(),
                info.product_id(),
                info.product_string().unwrap_or("")
            );
            if !seen.insert(key) {
                continue;
            }
            let product = info
                .product_string()
                .filter(|s| !s.is_empty())
                .unwrap_or("HID Battery Device")
                .to_string();
            let mfr = info.manufacturer_string().unwrap_or("").to_string();
            if is_system_battery(&mfr, &product) {
                continue;
            }
            let path_key = info.path().to_string_lossy().into_owned();
            let cached = layout_cache()
                .lock()
                .ok()
                .and_then(|cache| cache.get(&path_key).cloned());
            let scannable = battery_collection(info.usage_page());
            // Already known to declare no battery field, and no reason to scan
            // it either — skip the open entirely.
            if matches!(cached, Some(None)) && !scannable {
                continue;
            }

            let Ok(dev) = api.open_path(info.path()) else {
                continue;
            };

            let layout = match cached {
                Some(known) => known,
                None => {
                    let parsed = layout_of(&dev);
                    if let Ok(mut cache) = layout_cache().lock() {
                        cache.insert(path_key, parsed.clone());
                    }
                    parsed
                }
            };
            // Descriptor unavailable → assume numbered reports, matching the
            // feature-report convention, so the ID is never read as a percent.
            let uses_report_ids = layout.as_ref().map(|l| l.uses_report_ids).unwrap_or(true);

            let mut reading = match layout {
                Some(ref found) if found.has_battery() => {
                    read_from_layout(&dev, found, uses_report_ids)
                }
                _ => None,
            };
            if reading.is_none() && scannable {
                reading = read_heuristic(&dev, uses_report_ids);
            }

            if let Some((percent, charging)) = reading {
                let brand = Brand::identify(info.vendor_id(), &mfr, &product);
                let transport = hid::transport_label(info);
                out.push(
                    DeviceReading::ok(brand, product, transport, percent, charging)
                        .ranked(super::RANK_DESCRIPTOR),
                );
            }
        }
        out
    });
    result.unwrap_or_default()
}
