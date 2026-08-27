//! User-taught devices.
//!
//! Hardware that exposes no standard battery field still tends to publish the
//! state of charge as a plain byte inside a vendor feature report. Guessing
//! which byte is unsafe on its own — see `hid_battery` — but once the user has
//! confirmed the value they see on their device, the exact location can be
//! stored and read back precisely on every poll.

use super::hid;
use super::{Brand, DeviceReading};
use hidapi::DeviceInfo;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const STORE_FILE: &str = "devices.json";
const REPORT_MAX: usize = 64;
/// A state of charge moves. Nothing says how fast, so the window is generous:
/// a keyboard resting at 92% through a working day is ordinary. A vendor report
/// that has not changed one bit in six hours of polling is not a measurement —
/// it is a constant the scan mistook for a percentage, because two samples
/// 180 ms apart cannot tell the two apart.
const STALE_AFTER_MS: u64 = 6 * 60 * 60 * 1000;
/// Guards against flagging a device on the single read that follows a long
/// suspend, where the clock has moved but the hardware has barely been asked.
const STALE_AFTER_POLLS: u32 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedDevice {
    /// `VID:PID:USAGEPAGE:REPORTID:OFFSET`, stable across reboots.
    pub id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub report_id: u8,
    /// Byte index inside the report payload (after the leading report ID).
    pub byte_offset: usize,
    /// Raw value that means 100% — 0 falls back to a plain percentage.
    #[serde(default)]
    pub max_value: u8,
    /// USB interface the scan actually probed. `None` on entries taught before
    /// the interface was recorded; those still match on VID/PID/usage page.
    #[serde(default)]
    pub interface: Option<i32>,
    /// HID usage inside the collection, paired with `usage_page`.
    #[serde(default)]
    pub usage: Option<u16>,
    /// Evidence that the taught location really is telemetry: the report as it
    /// last read, and when it last differed. Persisted so the verdict survives
    /// restarts instead of starting over every launch.
    #[serde(default)]
    pub last_payload: Option<Vec<u8>>,
    #[serde(default)]
    pub last_change_ms: Option<u64>,
    #[serde(default, skip_serializing)]
    pub polls: u32,
}

impl LearnedDevice {
    pub fn key(vendor_id: u16, product_id: u16, usage_page: u16, report_id: u8, offset: usize) -> String {
        format!("{vendor_id:04X}:{product_id:04X}:{usage_page:04X}:{report_id:02X}:{offset}")
    }

    /// Every collection the taught byte could live in.
    fn matches(&self, info: &DeviceInfo) -> bool {
        info.vendor_id() == self.vendor_id
            && info.product_id() == self.product_id
            && info.usage_page() == self.usage_page
    }

    /// The one collection the scan read the byte from.
    fn is_exact(&self, info: &DeviceInfo) -> bool {
        self.interface
            .is_some_and(|number| number == info.interface_number())
            && self.usage.is_none_or(|usage| usage == info.usage())
    }

    fn percent(&self, raw: u8) -> Option<u8> {
        let max = if self.max_value == 0 { 100 } else { self.max_value };
        if max == 100 {
            return (raw <= 100).then_some(raw);
        }
        Some(((raw as u32 * 100) / max as u32).min(100) as u8)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn store_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)?;
    let dir = base.join("Battery Hub");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join(STORE_FILE))
}

fn cache() -> &'static Mutex<Vec<LearnedDevice>> {
    static CACHE: OnceLock<Mutex<Vec<LearnedDevice>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(load_from_disk()))
}

fn load_from_disk() -> Vec<LearnedDevice> {
    let Some(path) = store_path() else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_to_disk(devices: &[LearnedDevice]) -> Result<(), String> {
    let path = store_path().ok_or("No writable app data directory.")?;
    let json = serde_json::to_string_pretty(devices).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

pub fn all() -> Vec<LearnedDevice> {
    cache().lock().map(|list| list.clone()).unwrap_or_default()
}

/// Adding the same location twice just updates the name.
pub fn add(device: LearnedDevice) -> Result<Vec<LearnedDevice>, String> {
    let mut list = cache().lock().map_err(|_| "device list is poisoned")?;
    match list.iter_mut().find(|existing| existing.id == device.id) {
        Some(existing) => *existing = device,
        None => list.push(device),
    }
    save_to_disk(&list)?;
    Ok(list.clone())
}

pub fn remove(id: &str) -> Result<Vec<LearnedDevice>, String> {
    let mut list = cache().lock().map_err(|_| "device list is poisoned")?;
    list.retain(|device| device.id != id);
    save_to_disk(&list)?;
    Ok(list.clone())
}

/// Record what the report actually held this poll.
///
/// Returns whether the taught location still looks like a reading. The whole
/// report is compared, not just the taught byte: a live report almost always
/// carries something that moves next to the value — a counter, a status flag, a
/// checksum — while a frozen block of bytes is a constant the firmware answers
/// with and never updates.
fn note_observation(id: &str, payload: &[u8]) -> bool {
    let mut list = match cache().lock() {
        Ok(list) => list,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(device) = list.iter_mut().find(|device| device.id == id) else {
        return true;
    };

    let now = now_ms();
    let mut changed = false;
    if device.last_payload.as_deref() == Some(payload) {
        device.polls = device.polls.saturating_add(1);
    } else {
        device.last_payload = Some(payload.to_vec());
        device.last_change_ms = Some(now);
        device.polls = 0;
        changed = true;
    }

    let unchanged_for = device
        .last_change_ms
        .map_or(0, |since| now.saturating_sub(since));
    let verified = device.polls < STALE_AFTER_POLLS || unchanged_for < STALE_AFTER_MS;

    if changed {
        let snapshot = list.clone();
        drop(list);
        let _ = save_to_disk(&snapshot);
    }
    verified
}

/// Read every taught device. Ones that are not currently connected are
/// reported as absent so they drop out of the snapshot instead of going stale.
pub fn read_all() -> Vec<DeviceReading> {
    let devices = all();
    if devices.is_empty() {
        return Vec::new();
    }

    let result = hid::with_api(|api| {
        let mut out = Vec::with_capacity(devices.len());
        for device in &devices {
            // One product usually publishes several collections behind the same
            // usage page, and only one of them carries the report the scan read.
            // Picking whichever the enumeration happened to list first lands on
            // a static vendor report: a percentage that never moves again.
            let mut collections: Vec<&DeviceInfo> =
                api.device_list().filter(|info| device.matches(info)).collect();
            collections.sort_by_key(|info| !device.is_exact(info));

            let mut reading = None;
            for info in collections {
                let Ok(handle) = api.open_path(info.path()) else {
                    continue;
                };
                let mut buf = [0u8; REPORT_MAX];
                buf[0] = device.report_id;
                let Ok(read) = handle.get_feature_report(&mut buf) else {
                    continue;
                };
                // Byte 0 is the report ID; the payload starts after it. A read
                // that short means this collection does not answer the report
                // at all — slicing it would panic, and `panic = "abort"` in the
                // release profile takes the whole tray app down with it.
                let end = read.min(REPORT_MAX);
                if end <= 1 {
                    continue;
                }
                let payload = &buf[1..end];
                let Some(raw) = payload.get(device.byte_offset).copied() else {
                    continue;
                };
                if let Some(percent) = device.percent(raw) {
                    let verified = note_observation(&device.id, payload);
                    reading = Some(DeviceReading::taught(
                        Brand::identify(device.vendor_id, "", &device.name),
                        device.name.clone(),
                        hid::transport_label(info),
                        percent,
                        verified,
                    ));
                    break;
                }
            }
            out.push(reading.unwrap_or_else(|| {
                DeviceReading::failed(
                    Brand::identify(device.vendor_id, "", &device.name),
                    device.name.clone(),
                    "HID",
                    "Taught device is not connected.",
                    false,
                )
            }));
        }
        out
    });

    result.unwrap_or_default()
}
