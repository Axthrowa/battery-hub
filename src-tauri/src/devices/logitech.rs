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
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const SHORT_ID: u8 = 0x10;
const LONG_ID: u8 = 0x11;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;

const FEAT_BATTERY_STATUS: u16 = 0x1000;
const FEAT_UNIFIED_BATTERY: u16 = 0x1004;
const FEAT_DEVICE_NAME: u16 = 0x0005;
const SW_ID: u8 = 0x0A;
const VENDOR_PAGE_MIN: u16 = 0xFF00;

fn product_label(name: Option<&str>) -> String {
    name.filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Logitech".to_string())
}

/// The receiver's own product string is useless as a device label.
fn fallback_label(product: &str) -> String {
    if product.is_empty() || is_receiver_name(product) {
        "Logitech Wireless Device".to_string()
    } else {
        product.to_string()
    }
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
    matches!(dev.write(data), Ok(sent) if sent > 0)
}

/// Third byte of a HID++ 2.0 call: the function in the high nibble, the
/// software id in the low one. The reply echoes it whole, which is what
/// separates an answer from a notification the device sent unprompted.
fn call_byte(function: u8) -> u8 {
    (function << 4) | SW_ID
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
        // A reply echoes the software id it was asked with; a notification the
        // device sends on its own carries none. Matching on the function nibble
        // alone lets those through, and their payload lands in the same places
        // a battery answer would — which is how a mouse sitting at 100% came
        // back as 15% for a few polls after the real answer timed out.
        if body[2] != call_byte(fn_id & 0x0F) {
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
    msg[3] = call_byte(0x00);
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
    msg[3] = call_byte(0x01);

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
    let charging = matches!(status, 1..=3);
    Some((percent, charging))
}

/// 0x1000 Battery Level Status — percent or coarse bands.
fn read_battery_status(dev: &HidDevice, device_index: u8, feat_index: u8) -> Option<(u8, bool)> {
    let mut msg = [0u8; LONG_LEN];
    msg[0] = LONG_ID;
    msg[1] = device_index;
    msg[2] = feat_index;
    msg[3] = call_byte(0x00);

    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let body = read_matching(dev, LONG_ID, device_index, feat_index, 0x00, 600)?;
    let discharge = *body.get(3)?;
    let status = body.get(5).copied().unwrap_or(0);
    let charging = matches!(status, 1..=3);
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

/// Last real level per receiver slot.
///
/// The 2.4 GHz link drops the odd exchange, and reporting "no answer" on those
/// polls takes the card off the panel and puts it back a poll later. Holding
/// the last true level bridges that.
///
/// It used to be held for five minutes, because the older battery features on
/// the same receiver would answer in place of a sleeping mouse and the cache
/// was the only thing standing between that and the panel. `carries_unified`
/// now stops those at the source, so the cache is back to the small job it was
/// meant for — and a mouse that has actually been put away leaves the panel in
/// a minute rather than five.
const READING_TTL: Duration = Duration::from_secs(60);

struct LastReading {
    percent: u8,
    at: Instant,
}

fn reading_cache() -> &'static Mutex<HashMap<u8, LastReading>> {
    static CACHE: OnceLock<Mutex<HashMap<u8, LastReading>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Receiver slots seen to carry UnifiedBattery.
///
/// Which feature a device answers on is a property of the device, not of the
/// poll, so it is worth remembering — and it is the only way to tell a mouse
/// that has gone off the link from one whose feature lookup merely stumbled.
fn unified_slots() -> &'static Mutex<HashSet<u8>> {
    static SLOTS: OnceLock<Mutex<HashSet<u8>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn note_unified(device_index: u8) {
    if let Ok(mut slots) = unified_slots().lock() {
        slots.insert(device_index);
    }
}

/// Resolved model name per receiver slot.
///
/// Asking costs three HID++ round trips and the answer does not change while
/// the mouse stays paired, so it is asked once. That also means a mouse that
/// has fallen asleep keeps the name it gave when it was awake, instead of
/// whatever its silence spells out.
fn name_cache() -> &'static Mutex<HashMap<u8, String>> {
    static CACHE: OnceLock<Mutex<HashMap<u8, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_name(device_index: u8) -> Option<String> {
    name_cache().lock().ok()?.get(&device_index).cloned()
}

fn remember_name(device_index: u8, name: &str) {
    if let Ok(mut cache) = name_cache().lock() {
        cache.insert(device_index, name.to_string());
    }
}

fn carries_unified(device_index: u8) -> bool {
    unified_slots()
        .lock()
        .map(|slots| slots.contains(&device_index))
        .unwrap_or(false)
}

fn remember(device_index: u8, value: (u8, bool)) -> (u8, bool) {
    if let Ok(mut cache) = reading_cache().lock() {
        cache.insert(
            device_index,
            LastReading {
                percent: value.0,
                at: Instant::now(),
            },
        );
    }
    value
}

/// The level only — never the charging flag that came with it.
///
/// Falling silent is not a neutral event. Pulling the cable is one of the
/// commonest reasons a device stops answering, so a cached "charging" is wrong
/// at exactly the moment it gets served, and the panel goes on showing a bolt
/// and a finished charge for the whole five minutes. A level survives a nap
/// unchanged; a charging flag does not, so only the level is handed on.
fn recent(device_index: u8) -> Option<(u8, bool)> {
    reading_cache().lock().ok().and_then(|cache| {
        cache
            .get(&device_index)
            .filter(|last| last.at.elapsed() < READING_TTL)
            .map(|last| (last.percent, false))
    })
}

fn try_device_index(dev: &HidDevice, device_index: u8) -> Option<(u8, bool)> {
    // Which of the three answered is worth recording: they disagree, and when
    // one of them starts returning nonsense the log is the only way to tell
    // them apart afterwards.
    let note = |source: &str, value: (u8, bool)| {
        super::diagnostics::emit_line(&format!(
            "[hidpp] dev{device_index} {source} -> {}% charging={}",
            value.0, value.1
        ));
        value
    };

    // A device that carries UnifiedBattery is answered by it and nothing else.
    // The older features stay reachable on such hardware but do not describe it
    // any more: on a PRO X2 SUPERSTRIKE sitting at 100%, 0x1000 answers "15%,
    // charging". Falling through to it whenever 0x1004 misses a poll turned
    // that into a reading, and into a low-battery toast for a full mouse.
    if let Some(idx) = get_feature_index(dev, device_index, FEAT_UNIFIED_BATTERY) {
        note_unified(device_index);
        return read_unified_battery(dev, device_index, idx)
            .map(|v| remember(device_index, note("unified(0x1004)", v)))
            .or_else(|| recent(device_index));
    }
    if carries_unified(device_index) {
        // This slot has answered on 0x1004 before and cannot now, which means
        // the mouse is off the link — switched off, or asleep. The receiver
        // will still answer for it on 0x1000, and what it says there describes
        // no device: on a PRO X2 SUPERSTRIKE it is "15%, charging", whatever
        // the mouse is really doing. That is how a switched-off mouse ends up
        // on the panel as a nearly flat battery on the cable.
        super::diagnostics::emit_line(&format!(
            "[hidpp] dev{device_index} 0x1004 silent — mouse off the link, not falling back"
        ));
        return recent(device_index);
    }
    if let Some(idx) = get_feature_index(dev, device_index, FEAT_BATTERY_STATUS) {
        if let Some(v) = read_battery_status(dev, device_index, idx) {
            return Some(remember(device_index, note("status(0x1000)", v)));
        }
    }
    // 0x1001 is BatteryVoltage: millivolts, not a percentage, so no use here.
    read_hidpp10_battery(dev, device_index)
        .map(|v| remember(device_index, note("hid++1.0(0x0D)", v)))
        .or_else(|| recent(device_index))
}

/// 0x0005 DeviceNameAndType — the peripheral's own model name. A mouse paired
/// through a LIGHTSPEED receiver inherits the receiver's "USB Receiver" product
/// string over USB, so the name has to be asked for over HID++ instead.
fn read_device_name(dev: &HidDevice, device_index: u8) -> Option<String> {
    let feat_index = get_feature_index(dev, device_index, FEAT_DEVICE_NAME)?;

    let mut msg = [0u8; LONG_LEN];
    msg[0] = LONG_ID;
    msg[1] = device_index;
    msg[2] = feat_index;
    msg[3] = call_byte(0x00); // getCount
    drain(dev);
    if !write_msg(dev, &msg) {
        return None;
    }
    let body = read_matching(dev, LONG_ID, device_index, feat_index, 0x00, 600)?;
    let length = (*body.get(3)? as usize).min(64);
    if length == 0 {
        return None;
    }

    // getDeviceName returns up to 16 characters per call.
    let mut name = String::with_capacity(length);
    while name.len() < length {
        let mut msg = [0u8; LONG_LEN];
        msg[0] = LONG_ID;
        msg[1] = device_index;
        msg[2] = feat_index;
        msg[3] = call_byte(0x01); // getDeviceName
        msg[4] = name.len() as u8;
        drain(dev);
        if !write_msg(dev, &msg) {
            break;
        }
        let Some(body) = read_matching(dev, LONG_ID, device_index, feat_index, 0x01, 600) else {
            break;
        };
        let Some(chunk) = body.get(3..) else {
            break;
        };
        let before = name.len();
        for &byte in chunk {
            if name.len() >= length || byte == 0 {
                break;
            }
            if byte.is_ascii_graphic() || byte == b' ' {
                name.push(byte as char);
            }
        }
        // No progress means the device stopped answering — do not spin.
        if name.len() == before {
            break;
        }
    }

    let trimmed = name.trim();
    looks_like_a_name(trimmed).then(|| trimmed.to_string())
}

/// Whether what came back reads as a product name at all.
///
/// The loop above keeps whatever printable bytes arrive, and a receiver
/// answering for a mouse that has gone to sleep returns a frame filled with one
/// repeated byte. Here that byte is `0x77`, which is how a PRO X2 SUPERSTRIKE
/// came to sit on the panel labelled `wwwwwwww`.
fn looks_like_a_name(name: &str) -> bool {
    let mut rest = name.chars();
    let Some(first) = rest.next() else {
        return false;
    };
    name.len() >= 3 && rest.any(|c| c != first)
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
            let name = cached_name(index)
                .or_else(|| {
                    read_device_name(dev, index)
                        .filter(|found| !is_receiver_name(found))
                        .inspect(|found| remember_name(index, found))
                })
                .unwrap_or_else(|| fallback_label(product));
            return Some(
                DeviceReading::ok(Brand::classify("Logitech", &name), &name, "2.4 GHz", percent, charging)
                    .ranked(crate::devices::RANK_VENDOR)
                    .of_kind(crate::devices::DeviceKind::from_name(&name)),
            );
        }
    }
    None
}

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

#[cfg(test)]
mod tests {
    use super::looks_like_a_name;

    #[test]
    fn a_real_model_name_passes() {
        assert!(looks_like_a_name("PRO X2 SUPERSTRIKE"));
        assert!(looks_like_a_name("MX Master 3S"));
    }

    /// What a sleeping mouse's receiver actually returned: one byte, repeated.
    #[test]
    fn a_filled_buffer_is_not_a_name() {
        assert!(!looks_like_a_name("wwwwwwww"));
        assert!(!looks_like_a_name("\u{7f}\u{7f}\u{7f}\u{7f}"));
        assert!(!looks_like_a_name("        ".trim()));
    }

    #[test]
    fn nothing_and_near_nothing_are_refused() {
        assert!(!looks_like_a_name(""));
        assert!(!looks_like_a_name("K"));
        assert!(!looks_like_a_name("ab"), "two characters is not a product name");
    }
}
