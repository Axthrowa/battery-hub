use hidapi::{DeviceInfo, HidApi, HidDevice, HidResult};
use serde::Serialize;
use std::cmp::Reverse;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RAZER_VID: u16 = 0x1532;
const PID_DONGLE: u16 = 0x0565;
const PID_WIRED: u16 = 0x056E;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryReading {
    pub ok: bool,
    pub percent: Option<u8>,
    pub charging: bool,
    pub transport: String,
    pub product: String,
    pub error: Option<String>,
    /// Unix epoch milliseconds, so the UI can show when the value was actually
    /// measured instead of when it happened to receive it.
    pub updated_at_ms: u64,
}

impl BatteryReading {
    fn failed(transport: &str, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            percent: None,
            charging: false,
            transport: transport.into(),
            product: PRODUCT_NAME.into(),
            error: Some(error.into()),
            updated_at_ms: now_ms(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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
    let mut score = if info.product_id() == PID_DONGLE { 1 } else { 0 };
    match info.usage_page() {
        PREFERRED_USAGE_PAGE => score += 100,
        page if page >= VENDOR_USAGE_PAGE_MIN => score += 40,
        _ => {}
    }
    // Path shape is a weaker second opinion for the same collection.
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

fn try_open(dev: &HidDevice, product_id: u16) -> Option<BatteryReading> {
    let dongle = product_id == PID_DONGLE;

    let mut wake = [0u8; REPORT_LEN];
    wake[0] = RF_WAKE;
    let _ = write_report(dev, &wake);
    thread::sleep(Duration::from_millis(40));

    let _ = query_byte(dev, CMD_LINK, dongle, 500);

    let mut percent = query_byte(dev, CMD_BATTERY, dongle, 1200);
    if percent.is_none() && dongle {
        percent = query_byte(dev, CMD_BATTERY, false, 800);
    }
    let percent = percent?;
    let charging = query_byte(dev, CMD_CHARGING, dongle, 800).unwrap_or(0) > 0;

    Some(BatteryReading {
        ok: true,
        percent: Some(percent.min(100)),
        charging,
        transport: if dongle {
            "2.4 GHz".into()
        } else {
            "USB".into()
        },
        product: PRODUCT_NAME.into(),
        error: None,
        updated_at_ms: now_ms(),
    })
}

/// The hidapi context is kept alive between polls: only the device list is
/// rebuilt, and only for our VID/PID, so Windows is not asked to open and
/// describe every HID interface on the machine once a minute. The mutex also
/// serialises a manual refresh against the background poll, which would
/// otherwise race for the same HID path.
fn hid_context() -> &'static Mutex<Option<HidApi>> {
    static CONTEXT: OnceLock<Mutex<Option<HidApi>>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new(None))
}

fn refresh_device_list(api: &mut HidApi) -> HidResult<()> {
    api.reset_devices()?;
    api.add_devices(RAZER_VID, PID_DONGLE)?;
    api.add_devices(RAZER_VID, PID_WIRED)?;
    Ok(())
}

pub fn read_battery() -> BatteryReading {
    let mut guard = hid_context()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if guard.is_none() {
        match HidApi::new() {
            Ok(api) => *guard = Some(api),
            Err(e) => return BatteryReading::failed("", format!("HID init failed: {e}")),
        }
    }
    let api = guard.as_mut().expect("initialised above");
    if let Err(e) = refresh_device_list(api) {
        return BatteryReading::failed("", format!("HID enumeration failed: {e}"));
    }

    let mut ranked: Vec<(i32, CString, u16)> = api
        .device_list()
        .map(|d| (device_score(d), d.path().to_owned(), d.product_id()))
        .collect();
    ranked.sort_by_key(|(score, _, _)| Reverse(*score));

    if ranked.is_empty() {
        return BatteryReading::failed("", "Headset not found. Check 2.4 GHz dongle or Bluetooth.");
    }

    for (_, path, product_id) in &ranked {
        if let Ok(dev) = api.open_path(path) {
            if let Some(ok) = try_open(&dev, *product_id) {
                return ok;
            }
        }
    }

    BatteryReading::failed("2.4 GHz", "Dongle found but headset did not respond.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same vector the Python `--check-protocol` self-check asserts, so the
    /// Rust port cannot drift away from the framing the headset accepts.
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

    /// Opt-in end-to-end check: `cargo test --lib -- --ignored --nocapture`
    /// with the dongle plugged in and the headset powered on.
    #[test]
    #[ignore = "requires the 2.4 GHz dongle and a powered-on headset"]
    fn hardware_probe() {
        let reading = read_battery();
        println!("{reading:?}");
        assert!(reading.ok, "no reading: {:?}", reading.error);
        assert!(matches!(reading.percent, Some(p) if p <= 100));
    }

    #[test]
    fn accepts_a_leading_pad_byte_before_the_report_id() {
        let mut padded = vec![0x00];
        padded.extend_from_slice(&reply(CMD_BATTERY, 0x01, 42));
        assert_eq!(parse_reply(&padded, CMD_BATTERY), Some(42));
    }
}
