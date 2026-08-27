//! Razer BlackShark V2 HyperSpeed — vendor HID over the HyperSpeed dongle.

use super::hid::{self, RAZER_PID_DONGLE, RAZER_PID_WIRED, RAZER_VID};
use super::{Brand, DeviceReading};
use hidapi::{DeviceInfo, HidDevice};
use std::cmp::Reverse;
use std::ffi::CString;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const REPORT_LEN: usize = 64;
const REPORT_ID: u8 = 0x02;
const RF_WAKE: u8 = 0x05;
const CHANNEL: u8 = 0x60;
const CRC_INDEX: usize = 62;
const CLASS_HEADSET: u8 = 0x80;
const CMD_BATTERY: u8 = 0x21;
const CMD_CHARGING: u8 = 0x2A;
const CMD_LINK: u8 = 0x20;

/// Windows splits the dongle's MI_03 interface into several top-level
/// collections; battery replies only ever arrive on the 0xFF14 vendor page
/// (Col04 in the device path), never on 0xFF13.
const PREFERRED_USAGE_PAGE: u16 = 0xFF14;
const VENDOR_USAGE_PAGE_MIN: u16 = 0xFF00;

const PRODUCT_NAME: &str = "Razer BlackShark V2 HyperSpeed";

fn xor_checksum(buf: &[u8]) -> u8 {
    buf[..CRC_INDEX].iter().fold(0u8, |crc, byte| crc ^ byte)
}

fn build_query(cmd: u8, dongle: bool) -> [u8; REPORT_LEN] {
    let mut buf = [0u8; REPORT_LEN];
    buf[0] = REPORT_ID;
    buf[2] = CHANNEL;
    buf[6] = 0x04;
    buf[10] = cmd;
    buf[12] = 0x00;
    if dongle {
        buf[9] = CLASS_HEADSET;
        buf[CRC_INDEX] = xor_checksum(&buf);
    }
    buf
}

fn parse_reply(data: &[u8], expected_cmd: u8) -> Option<u8> {
    // hidapi may or may not prefix the report id depending on the collection.
    let payload = if data.first() == Some(&REPORT_ID) {
        data
    } else if data.len() > 14 && data[1] == REPORT_ID {
        &data[1..]
    } else {
        return None;
    };
    if payload.len() <= 13 || payload[10] != expected_cmd || payload[11] != 0x01 {
        return None;
    }
    Some(payload[13])
}

fn device_score(info: &DeviceInfo) -> i32 {
    let mut score = if info.product_id() == RAZER_PID_DONGLE { 1 } else { 0 };
    match info.usage_page() {
        PREFERRED_USAGE_PAGE => score += 100,
        page if page >= VENDOR_USAGE_PAGE_MIN => score += 40,
        _ => {}
    }
    let path = info.path().to_string_lossy().to_ascii_lowercase();
    if path.contains("col04") {
        score += 10;
    } else if path.contains("mi_03") {
        score += 4;
    }
    score
}

fn write_report(dev: &HidDevice, data: &[u8]) -> bool {
    match dev.write(data) {
        Ok(n) if n > 0 => true,
        _ => dev.send_feature_report(data).is_ok(),
    }
}

fn drain(dev: &HidDevice) {
    let mut buf = [0u8; REPORT_LEN + 1];
    for _ in 0..24 {
        match dev.read_timeout(&mut buf, 5) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn query_byte(dev: &HidDevice, cmd: u8, dongle: bool, timeout_ms: u64) -> Option<u8> {
    let report = build_query(cmd, dongle);
    drain(dev);
    if !write_report(dev, &report) {
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut buf = [0u8; REPORT_LEN + 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let wait = (remaining.as_millis() as i32).clamp(1, 250);
        if let Ok(n) = dev.read_timeout(&mut buf, wait) {
            if n > 0 {
                if let Some(v) = parse_reply(&buf[..n], cmd) {
                    return Some(v);
                }
            }
        }
    }
}

/// Consecutive polls the headset has stayed silent.
///
/// A dongle left plugged into a desktop is the normal case, and a headset that
/// is switched off answers none of these queries: at full timeouts that is
/// three seconds burned on every poll, which is most of what the refresh button
/// waits for. Once it is clearly off the same queries are asked briefly, and a
/// single answer puts the full budget back.
static SILENT_POLLS: AtomicU32 = AtomicU32::new(0);
const SILENT_POLLS_BEFORE_BACKOFF: u32 = 3;

/// Milliseconds to wait for one reply, given how long the headset has been off.
fn budget(full: u64) -> u64 {
    if SILENT_POLLS.load(Ordering::Relaxed) >= SILENT_POLLS_BEFORE_BACKOFF {
        (full / 8).max(60)
    } else {
        full
    }
}

fn try_open(dev: &HidDevice, product_id: u16) -> Option<DeviceReading> {
    let dongle = product_id == RAZER_PID_DONGLE;

    let mut wake = [0u8; REPORT_LEN];
    wake[0] = RF_WAKE;
    let _ = write_report(dev, &wake);
    thread::sleep(Duration::from_millis(40));

    let _ = query_byte(dev, CMD_LINK, dongle, budget(500));

    let mut percent = query_byte(dev, CMD_BATTERY, dongle, budget(1200));
    if percent.is_none() && dongle {
        percent = query_byte(dev, CMD_BATTERY, false, budget(800));
    }
    let percent = percent?;
    let charging = query_byte(dev, CMD_CHARGING, dongle, budget(800)).unwrap_or(0) > 0;

    Some(
        DeviceReading::ok(
            Brand::razer(),
            PRODUCT_NAME,
            if dongle { "2.4 GHz" } else { "USB" },
            percent.min(100),
            charging,
        )
        .ranked(crate::devices::RANK_VENDOR),
    )
}

/// Cheap presence probe: enumerates our VID/PIDs without talking to the headset.
pub fn dongle_present() -> bool {
    hid::with_api(|api| {
        api.device_list()
            .any(|d| d.vendor_id() == RAZER_VID && matches!(d.product_id(), RAZER_PID_DONGLE | RAZER_PID_WIRED))
    })
    .unwrap_or(false)
}

pub fn read() -> DeviceReading {
    let ranked = match hid::with_api(|api| {
        let mut ranked: Vec<(i32, CString, u16)> = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == RAZER_VID
                    && matches!(d.product_id(), RAZER_PID_DONGLE | RAZER_PID_WIRED)
            })
            .map(|d| (device_score(d), d.path().to_owned(), d.product_id()))
            .collect();
        ranked.sort_by_key(|(score, _, _)| Reverse(*score));
        ranked
    }) {
        Ok(v) => v,
        Err(e) => return DeviceReading::failed(Brand::razer(), PRODUCT_NAME, "", e, false),
    };

    if ranked.is_empty() {
        return DeviceReading::failed(
            Brand::razer(),
            PRODUCT_NAME,
            "",
            "Headset not found. Check 2.4 GHz dongle or Bluetooth.",
            false,
        );
    }

    let present = true;
    let opened = hid::with_api(|api| {
        for (_, path, product_id) in &ranked {
            if let Ok(dev) = api.open_path(path) {
                if let Some(ok) = try_open(&dev, *product_id) {
                    return Some(ok);
                }
            }
        }
        None
    });

    SILENT_POLLS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |seen| {
        Some(if matches!(opened, Ok(Some(_))) { 0 } else { seen.saturating_add(1) })
    })
    .ok();

    match opened {
        Ok(Some(ok)) => ok,
        Ok(None) => DeviceReading::failed(
            Brand::razer(),
            PRODUCT_NAME,
            "2.4 GHz",
            "Dongle found but headset did not respond.",
            present,
        ),
        Err(e) => DeviceReading::failed(Brand::razer(), PRODUCT_NAME, "", e, present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dongle_battery_query_matches_known_frame() {
        let report = build_query(CMD_BATTERY, true);
        assert_eq!(report[0], 0x02);
        assert_eq!(report[2], 0x60);
        assert_eq!(report[6], 0x04);
        assert_eq!(report[9], 0x80);
        assert_eq!(report[10], CMD_BATTERY);
        assert_eq!(report[CRC_INDEX], xor_checksum(&report));
        assert_eq!(report[CRC_INDEX], 0xC7);
    }

    #[test]
    fn wired_query_carries_no_class_or_checksum() {
        let report = build_query(CMD_BATTERY, false);
        assert_eq!(report[9], 0x00);
        assert_eq!(report[CRC_INDEX], 0x00);
    }

    fn reply(cmd: u8, ack: u8, value: u8) -> [u8; REPORT_LEN] {
        let mut buf = [0u8; REPORT_LEN];
        buf[0] = REPORT_ID;
        buf[10] = cmd;
        buf[11] = ack;
        buf[12] = 0x01;
        buf[13] = value;
        buf
    }

    #[test]
    fn parses_acked_reply() {
        assert_eq!(parse_reply(&reply(CMD_BATTERY, 0x01, 73), CMD_BATTERY), Some(73));
    }

    #[test]
    fn rejects_wrong_command_missing_ack_and_short_frames() {
        assert_eq!(parse_reply(&reply(CMD_CHARGING, 0x01, 73), CMD_BATTERY), None);
        assert_eq!(parse_reply(&reply(CMD_BATTERY, 0x00, 73), CMD_BATTERY), None);
        assert_eq!(parse_reply(&[], CMD_BATTERY), None);
        assert_eq!(parse_reply(&[REPORT_ID, 0, 0], CMD_BATTERY), None);
    }

    #[test]
    fn accepts_a_leading_pad_byte_before_the_report_id() {
        let mut padded = vec![0x00];
        padded.extend_from_slice(&reply(CMD_BATTERY, 0x01, 42));
        assert_eq!(parse_reply(&padded, CMD_BATTERY), Some(42));
    }
}
