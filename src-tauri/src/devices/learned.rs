//! User-taught devices.
//!
//! Hardware that exposes no standard battery field still tends to publish the
//! state of charge as a plain byte inside a vendor feature report. Guessing
//! which byte is unsafe on its own — see `hid_battery` — but once the user has
//! confirmed the value they see on their device, the exact location can be
//! stored and read back precisely on every poll.

use super::hid;
use super::{Brand, DeviceReading};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const STORE_FILE: &str = "devices.json";
const REPORT_MAX: usize = 64;

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
}

impl LearnedDevice {
    pub fn key(vendor_id: u16, product_id: u16, usage_page: u16, report_id: u8, offset: usize) -> String {
        format!("{vendor_id:04X}:{product_id:04X}:{usage_page:04X}:{report_id:02X}:{offset}")
    }

    fn percent(&self, raw: u8) -> Option<u8> {
        let max = if self.max_value == 0 { 100 } else { self.max_value };
        if max == 100 {
            return (raw <= 100).then_some(raw);
        }
        Some(((raw as u32 * 100) / max as u32).min(100) as u8)
    }
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
            let mut reading = None;
            for info in api.device_list() {
                if info.vendor_id() != device.vendor_id
                    || info.product_id() != device.product_id
                    || info.usage_page() != device.usage_page
                {
                    continue;
                }
                let Ok(handle) = api.open_path(info.path()) else {
                    continue;
                };
                let mut buf = [0u8; REPORT_MAX];
                buf[0] = device.report_id;
                let Ok(read) = handle.get_feature_report(&mut buf) else {
                    continue;
                };
                // Byte 0 is the report ID; the payload starts after it.
                let payload = &buf[1..read.min(REPORT_MAX)];
                let Some(raw) = payload.get(device.byte_offset).copied() else {
                    continue;
                };
                if let Some(percent) = device.percent(raw) {
                    reading = Some(DeviceReading::ok(
                        Brand::classify("", &device.name),
                        device.name.clone(),
                        "HID",
                        percent,
                        false,
                    ));
                    break;
                }
            }
            out.push(reading.unwrap_or_else(|| {
                DeviceReading::failed(
                    Brand::classify("", &device.name),
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
