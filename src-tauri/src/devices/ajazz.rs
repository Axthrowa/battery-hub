//! Ajazz 2.4G 8K (VID=0x3151 PID=0x5007) battery via vendor status report.
//!
//! 1. Open collection usage_page=0xFFFF usage=0x02 (MI_02)
//! 2. SET_FEATURE `[0x00, 0xF7, …]` — wakes 2.4 GHz telemetry
//! 3. Wait ~50–80 ms
//! 4. GET_FEATURE report `0x05` → `05 00 00 NN …` (NN = percent on Windows)

use super::hid::{self, AJAZZ_VIDS};
use super::{Brand, DeviceReading};
use hidapi::{DeviceInfo, HidDevice};
use std::cmp::Reverse;
use std::ffi::CString;
use std::thread;
use std::time::Duration;

const PRODUCT_FALLBACK: &str = "AJAZZ 2.4G 8K";

/// The 2.4 GHz receivers all report the same generic OEM product string, so
/// known models are labelled by VID/PID instead.
const KNOWN_MODELS: &[(u16, u16, &str)] = &[(0x3151, 0x5007, "Ajazz AJ159 Apex")];

fn model_label(info: &DeviceInfo) -> String {
    if let Some((_, _, name)) = KNOWN_MODELS
        .iter()
        .find(|(vid, pid, _)| *vid == info.vendor_id() && *pid == info.product_id())
    {
        return (*name).to_string();
    }
    info.product_string()
        .filter(|s| !s.is_empty())
        .unwrap_or(PRODUCT_FALLBACK)
        .to_string()
}

fn name_looks_ajazz(info: &DeviceInfo) -> bool {
    let product = info.product_string().unwrap_or("").to_ascii_lowercase();
    let manufacturer = info.manufacturer_string().unwrap_or("").to_ascii_lowercase();
    let vid = info.vendor_id();
    let pid = info.product_id();
    product.contains("ajazz")
        || manufacturer.contains("ajazz")
        || AJAZZ_VIDS.contains(&vid)
        || (crate::devices::hid::AJAZZ_VID_PRIMARY != 0
            && vid == crate::devices::hid::AJAZZ_VID_PRIMARY)
        || (vid == 0x3151 && pid == 0x5007)
}

fn device_score(info: &DeviceInfo) -> i32 {
    let mut score = 0;
    match (info.usage_page(), info.usage()) {
        (0xFFFF, 0x0002) => score += 200,
        (0xFFFF, _) => score += 80,
        (page, _) if page >= 0xFF00 => score += 40,
        _ => {}
    }
    let path = info.path().to_string_lossy().to_ascii_lowercase();
    if path.contains("mi_02") {
        score += 30;
    }
    score
}

fn parse_report05(buf: &[u8]) -> Option<u8> {
    if buf.len() < 4 {
        return None;
    }
    let (pad0, pad1, charge) = if buf[0] == 0x05 {
        (buf[1], buf[2], buf[3])
    } else {
        (buf[0], buf[1], buf[2])
    };
    if pad0 != 0 || pad1 != 0 {
        return None;
    }
    if (1..=100).contains(&charge) {
        Some(charge)
    } else {
        None
    }
}

fn read_aj_series_battery(dev: &HidDevice) -> Option<u8> {
    for attempt in 0..4 {
        let mut poll = [0u8; 65];
        poll[0] = 0x00;
        poll[1] = 0xF7;
        let sent = dev.send_feature_report(&poll).is_ok()
            || {
                let mut short = [0u8; 9];
                short[0] = 0x00;
                short[1] = 0xF7;
                dev.send_feature_report(&short).is_ok()
            }
            || {
                // Fallback: OUTPUT write of the same frame.
                let mut out = [0u8; 9];
                out[0] = 0x00;
                out[1] = 0xF7;
                matches!(dev.write(&out), Ok(n) if n > 0)
            };

        if !sent && attempt == 0 {
            // Still try a naked GET — sometimes telemetry is already up.
        }

        thread::sleep(Duration::from_millis(50 + attempt as u64 * 30));

        let mut buf = [0u8; 65];
        buf[0] = 0x05;
        if let Ok(n) = dev.get_feature_report(&mut buf) {
            if let Some(pct) = parse_report05(&buf[..n.max(4)]) {
                return Some(pct);
            }
        }
    }
    None
}

#[allow(dead_code)]
pub fn receiver_present() -> bool {
    hid::with_api(|api| api.device_list().any(name_looks_ajazz)).unwrap_or(false)
}

pub fn read() -> DeviceReading {
    // Enumerate + open + probe inside ONE with_api lock so reset_devices
    // cannot invalidate paths between list and open.
    match hid::with_api(|api| {
        let mut ranked: Vec<(i32, CString, String)> = api
            .device_list()
            .filter(|d| name_looks_ajazz(d))
            .map(|d| {
                (
                    device_score(d),
                    d.path().to_owned(),
                    model_label(d),
                )
            })
            .collect();
        ranked.sort_by_key(|(score, _, _)| Reverse(*score));

        if ranked.is_empty() {
            return DeviceReading::failed(
                Brand::ajazz(),
                PRODUCT_FALLBACK,
                "",
                "No Ajazz 2.4 GHz receiver found.",
                false,
            );
        }

        for (_, path, product) in &ranked {
            if let Ok(dev) = api.open_path(path) {
                if let Some(percent) = read_aj_series_battery(&dev) {
                    let brand = Brand::classify("", product);
                    return DeviceReading::ok(brand, product, "2.4 GHz", percent, false)
                        .ranked(crate::devices::RANK_VENDOR)
                        .of_kind(crate::devices::DeviceKind::Mouse);
                }
            }
        }

        let label = ranked
            .first()
            .map(|(_, _, product)| product.clone())
            .unwrap_or_else(|| PRODUCT_FALLBACK.to_string());
        DeviceReading::failed(
            Brand::ajazz(),
            label,
            "2.4 GHz",
            "Receiver found (0x3151:0x5007) but 0xF7/0x05 battery poll failed (is the mouse on?).",
            true,
        )
    }) {
        Ok(r) => r,
        Err(e) => DeviceReading::failed(Brand::ajazz(), PRODUCT_FALLBACK, "", e, false),
    }
}
