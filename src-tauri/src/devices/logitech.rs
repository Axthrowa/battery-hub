//! Logitech HID++ battery reader.
//!
//! PRO X2 SUPERSTRIKE (and similar) often expose a direct HID++ interface
//! (e.g. PID 0xC0A8) alongside the Lightspeed receiver (0xC54D). Accurate %
//! comes from UnifiedBattery (0x1004) function 1 on the long report collection
//! (usage 2). Reading only the receiver and treating HID++ error bytes as %
//! previously produced a false ~13% (register echo 0x0D).

use super::hid::{self, LOGITECH_VID};
use super::{Brand, DeviceReading};
use hidapi::{DeviceInfo, HidDevice};
use std::cmp::Reverse;
use std::ffi::CString;
use std::time::{Duration, Instant};

const SHORT_ID: u8 = 0x10;
const LONG_ID: u8 = 0x11;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;

const FEAT_BATTERY_STATUS: u16 = 0x1000;
const FEAT_BATTERY_VOLTAGE: u16 = 0x1001;
const FEAT_UNIFIED_BATTERY: u16 = 0x1004;
const SW_ID: u8 = 0x0A;
const VENDOR_PAGE_MIN: u16 = 0xFF00;

fn product_label(name: Option<&str>) -> String {
    name.filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Logitech".to_string())
}

fn is_receiver_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("receiver") || n.contains("unifying") || n == "usb receiver"
}

fn is_hidpp_interface(usage_page: u16, usage: u16) -> bool {
    (usage_page == 0xFF00 && (usage == 1 || usage == 2)) || usage_page > VENDOR_PAGE_MIN
}

fn device_score(info: &DeviceInfo) -> i32 {
    let mut score = 0;
    let product = info.product_string().unwrap_or("");
    // Prefer the peripheral's own HID++ interface over the USB receiver.
    if !product.is_empty() && !is_receiver_name(product) {
        score += 200;
    }
    match (info.usage_page(), info.usage()) {
        // Long report (0x11) — required for HID++ 2.0 feature calls on Win.
        (0xFF00, 2) => score += 100,
        (0xFF00, 1) => score += 40,
        (page, _) if page >= VENDOR_PAGE_MIN => score += 20,
        _ => {}
    }
    let path = info.path().to_string_lossy().to_ascii_lowercase();
    if path.contains("mi_02") {
        score += 15;
    }
    if path.contains("col02") {
        score += 10;
    } else if path.contains("col01") {
        score += 2;
    }
    // Known direct mouse PID on this machine.
    if info.product_id() == 0xC0A8 {
        score += 50;
    }
    score
}

fn pid_allowed(pid: u16) -> bool {
    match hid::LOGITECH_PID_OVERRIDE {
        Some(only) => pid == only,
        None => {
            hid::LOGITECH_PIDS.contains(&pid)
                || pid == 0xC0A8
                || (0xC000..=0xC0FF).contains(&pid)
                || (0xC500..=0xC5FF).contains(&pid)
        }
    }
}

fn drain(dev: &HidDevice) {
    let mut buf = [0u8; 64];
    for _ in 0..24 {
        match dev.read_timeout(&mut buf, 5) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn write_msg(dev: &HidDevice, data: &[u8]) -> bool {
    match dev.write(data) {
        Ok(n) if n > 0 => true,
        _ => false,
    }
}

fn read_matching(
    dev: &HidDevice,
    report_id: u8,
    device_index: u8,
    feature_index: u8,
    fn_id: u8,
    timeout_ms: u64,
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 64];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let wait = (remaining.as_millis() as i32).clamp(1, 200);
        let Ok(n) = dev.read_timeout(&mut buf, wait) else {
            continue;
        };
        if n == 0 {
            continue;
        }
        let msg = &buf[..n];
        let (id, body) = if msg[0] == report_id {
            (msg[0], &msg[1..])
        } else if matches!(report_id, LONG_ID | SHORT_ID) && msg[0] != 0x00 {
            (report_id, msg)
        } else if msg.len() > 1 && msg[1] == report_id {
            (msg[1], &msg[2..])
        } else {
            continue;
        };
        if id != report_id || body.len() < 3 {
            continue;
        }
        // HID++ error frame
        if body[1] == 0x8F {
            return None;
        }
        if body[0] != device_index || body[1] != feature_index {
            continue;
        }
        if (body[2] >> 4) != (fn_id & 0x0F) {
            continue;
        }
        return Some(body.to_vec());
    }
}

fn get_feature_index(dev: &HidDevice, device_index: u8, feature_id: u16) -> Option<u8> {
    let mut msg = [0u8; LONG_LEN];
    msg[0] = LONG_ID;
    msg[1] = device_index;
    msg[2] = 0x00;
    msg[3] = (0x00 << 4) | SW_ID;
    msg[4] = (feature_id >> 8) as u8;
    msg[5] = (feature_id & 0xFF) as u8;

    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let body = read_matching(dev, LONG_ID, device_index, 0x00, 0x00, 600)?;
    let index = *body.get(3)?;
    // Root is index 0 — a real feature must map elsewhere.
    if index == 0 {
        return None;
    }
    Some(index)
}

/// 0x1004 UnifiedBattery.get_battery_info (function 1).
fn read_unified_battery(dev: &HidDevice, device_index: u8, feat_index: u8) -> Option<(u8, bool)> {
    let mut msg = [0u8; LONG_LEN];
    msg[0] = LONG_ID;
    msg[1] = device_index;
    msg[2] = feat_index;
    msg[3] = (0x01 << 4) | SW_ID;

    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let body = read_matching(dev, LONG_ID, device_index, feat_index, 0x01, 700)?;
    let percent = *body.get(3)?;
    if percent == 0 || percent > 100 {
        return None;
    }
    let status = body.get(5).copied().unwrap_or(0);
    // 1=charging, 2=charging nearly full, 3=charge complete (common mapping)
    let charging = matches!(status, 1 | 2 | 3);
    Some((percent, charging))
}

/// 0x1000 Battery Level Status — percent or coarse bands.
fn read_battery_status(dev: &HidDevice, device_index: u8, feat_index: u8) -> Option<(u8, bool)> {
    let mut msg = [0u8; LONG_LEN];
    msg[0] = LONG_ID;
    msg[1] = device_index;
    msg[2] = feat_index;
    msg[3] = (0x00 << 4) | SW_ID;

    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let body = read_matching(dev, LONG_ID, device_index, feat_index, 0x00, 600)?;
    let discharge = *body.get(3)?;
    let status = body.get(5).copied().unwrap_or(0);
    let charging = matches!(status, 1 | 2 | 3);
    if !(1..=100).contains(&discharge) {
        let pct = match discharge {
            0 => 5,
            1 => 20,
            2 => 40,
            3 => 60,
            4 => 80,
            5..=7 => 100,
            _ => return None,
        };
        return Some((pct, charging));
    }
    Some((discharge, charging))
}

/// HID++ 1.0 GetRegister(0x0D) — only accept real replies, never error frames.
fn read_hidpp10_battery(dev: &HidDevice, device_index: u8) -> Option<(u8, bool)> {
    let mut msg = [0u8; SHORT_LEN];
    msg[0] = SHORT_ID;
    msg[1] = device_index;
    msg[2] = 0x81;
    msg[3] = 0x0D;
    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(400);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        let Ok(n) = dev.read_timeout(&mut buf, 80) else {
            continue;
        };
        if n < 5 {
            continue;
        }
        let body = if buf[0] == SHORT_ID || buf[0] == LONG_ID {
            &buf[1..n]
        } else {
            &buf[..n]
        };
        if body.len() < 4 {
            continue;
        }
        if body[0] != device_index && body[0] != 0xFF {
            continue;
        }
        // Error: [di, 0x8F, ...] — do NOT treat 0x0D address echo as percent.
        if body[1] == 0x8F {
            return None;
        }
        // Expected: [di, 0x81, 0x0D, discharge, next, status]
        if body[1] != 0x81 || body[2] != 0x0D {
            continue;
        }
        let level = body[3];
        if (1..=100).contains(&level) {
            let status = body.get(5).copied().unwrap_or(0);
            let charging = status != 0;
            return Some((level, charging));
        }
    }
    None
}

fn try_device_index(dev: &HidDevice, device_index: u8) -> Option<(u8, bool)> {
    if let Some(idx) = get_feature_index(dev, device_index, FEAT_UNIFIED_BATTERY) {
        if let Some(v) = read_unified_battery(dev, device_index, idx) {
            return Some(v);
        }
    }
    if let Some(idx) = get_feature_index(dev, device_index, FEAT_BATTERY_STATUS) {
        if let Some(v) = read_battery_status(dev, device_index, idx) {
            return Some(v);
        }
    }
    // 0x1001 is voltage — skip as percent source.
    let _ = FEAT_BATTERY_VOLTAGE;
    read_hidpp10_battery(dev, device_index)
}

fn probe_device(dev: &HidDevice, product: &str) -> Option<DeviceReading> {
    // Direct peripherals answer on 0xFF; receiver-attached slots use 1..6.
    let indices: &[u8] = if is_receiver_name(product) {
        &[1, 2, 3, 4, 5, 6, 0xFF]
    } else {
        &[0xFF, 1, 0, 2, 3, 4, 5, 6]
    };
    for &index in indices {
        if let Some((percent, charging)) = try_device_index(dev, index) {
            return Some(DeviceReading::ok(
                Brand::classify("Logitech", product),
                product,
                "2.4 GHz",
                percent,
                charging,
            ));
        }
    }
    None
}

#[allow(dead_code)]
pub fn receiver_present() -> bool {
    hid::with_api(|api| {
        api.device_list().any(|d| {
            d.vendor_id() == LOGITECH_VID
                && pid_allowed(d.product_id())
                && is_hidpp_interface(d.usage_page(), d.usage())
        })
    })
    .unwrap_or(false)
}

pub fn read() -> DeviceReading {
    let ranked: Vec<(i32, CString, String)> = match hid::with_api(|api| {
        let mut ranked: Vec<(i32, CString, String)> = api
            .device_list()
            .filter(|d| d.vendor_id() == LOGITECH_VID)
            .filter(|d| pid_allowed(d.product_id()))
            .filter(|d| is_hidpp_interface(d.usage_page(), d.usage()))
            .map(|d| {
                (
                    device_score(d),
                    d.path().to_owned(),
                    product_label(d.product_string()),
                )
            })
            .collect();
        ranked.sort_by_key(|(score, _, _)| Reverse(*score));
        ranked
    }) {
        Ok(v) => v,
        Err(e) => return DeviceReading::failed(Brand::logitech(), "Logitech", "", e, false),
    };

    if ranked.is_empty() {
        return DeviceReading::failed(
            Brand::logitech(),
            "Logitech",
            "",
            "No Logitech HID++ device found.",
            false,
        );
    }

    let present = true;
    let result = hid::with_api(|api| {
        for (_, path, product) in &ranked {
            if let Ok(dev) = api.open_path(path) {
                if let Some(ok) = probe_device(&dev, product) {
                    return Some(ok);
                }
            }
        }
        None
    });

    match result {
        Ok(Some(ok)) => ok,
        Ok(None) => DeviceReading::failed(
            Brand::logitech(),
            ranked
                .first()
                .map(|(_, _, p)| p.as_str())
                .unwrap_or("Logitech"),
            "2.4 GHz",
            "HID++ device found but battery feature did not respond.",
            present,
        ),
        Err(e) => DeviceReading::failed(Brand::logitech(), "Logitech", "2.4 GHz", e, present),
    }
}
