//! Aula / Compx keyboards — vendor battery frame.
//!
//! These keyboards publish no battery field anywhere: every collection they
//! expose is input-only or an opaque vendor pipe, which is why both the
//! automatic readers and the byte scan come up empty on them. On an F75 the
//! scan's only steady candidate is a firmware constant that never moves, so a
//! taught byte there reports the same percentage forever.
//!
//! The real state of charge sits behind a request/response frame, taken from
//! the protocol Aula's own WebHID configurator speaks:
//!
//!   TX  report 0x13, 19 bytes: `[0x4A, 0, 0, 0, …, crc]`
//!   RX  report 0x13, 19 bytes: `[0x4A, packets, index, len, percent, status, …, crc]`
//!
//! `crc` is the low byte of the report ID plus every byte before it — the same
//! sum on both directions, so a reply can be checked before it is believed.
//!
//! Verified against an Aula F75 receiver (VID 0x3554 PID 0xFA09):
//! TX `4A 00 … 5D` → RX `4A 01 00 02 50 01 … B1`, i.e. 80%.

use super::diagnostics;
use super::hid;
use super::{Brand, DeviceReading, RANK_VENDOR};
use hidapi::{DeviceInfo, HidDevice};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// OEM vendor IDs the configurator ships with. A frame is only ever written to
/// one of these, and only to its 19-byte vendor collection — nothing is sent
/// speculatively to hardware that merely happens to be plugged in.
pub const AULA_VIDS: &[u16] = &[
    0x3554, // Compx 2.4 GHz receiver — Aula F75 and siblings
    0x258A, // wired keyboards
    0x372E, // AULA KG 138 / HERO 84 HE
];

const VENDOR_USAGE_PAGE: u16 = 0xFF02;
const REPORT_ID: u8 = 0x13;
const PAYLOAD_LEN: usize = 19;
const CMD_BATTERY: u8 = 0x4A;
const CMD_UUID: u8 = 0x05;
/// The receiver drops a frame now and then when queries arrive back to back.
const RETRIES: u32 = 4;
const REPLY_WINDOW: Duration = Duration::from_millis(400);
const RETRY_GAP: Duration = Duration::from_millis(60);
const READ_SLICE_MS: i32 = 50;

/// The 2.4 GHz receiver reports one generic product string for every keyboard
/// that pairs with it, so the model can only come from the keyboard itself.
/// Identifiers are the configurator's own device table.
const KNOWN_MODELS: &[(u64, &str)] = &[
    (3298534883492, "Aula F99"),
    (3298534883533, "Aula F75"),
    (3298534884188, "Aula F75"),
    (3298534883595, "Aula F87"),
    (3298534883735, "GravaStar Mercury K1 PRO"),
    (3298534883764, "GravaStar Mercury K1"),
    (3298534883765, "GravaStar Mercury K1 LITE"),
    (18691697672204, "Aula KG 138"),
    (18691697672197, "Hero 84 HE"),
];

/// A resolved model is a property of the keyboard, not of the poll, so the
/// extra frame is spent once per device rather than once a minute.
fn model_cache() -> &'static Mutex<HashMap<(u16, u16), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(u16, u16), String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn checksum(bytes: &[u8]) -> u8 {
    let sum = bytes
        .iter()
        .fold(REPORT_ID as u32, |acc, byte| acc + *byte as u32);
    (sum % 256) as u8
}

fn frame(command: &[u8]) -> [u8; PAYLOAD_LEN] {
    let mut payload = [0u8; PAYLOAD_LEN];
    payload[..command.len()].copy_from_slice(command);
    payload[PAYLOAD_LEN - 1] = checksum(&payload[..PAYLOAD_LEN - 1]);
    payload
}

/// Send one command and return the answer's data bytes.
///
/// The reply is only accepted when it echoes the opcode and its own checksum
/// adds up, so an unrelated report sitting in the queue cannot be read as a
/// state of charge.
fn ask(dev: &HidDevice, command: &[u8], marker: u8) -> Option<Vec<u8>> {
    let payload = frame(command);
    let mut out = Vec::with_capacity(PAYLOAD_LEN + 1);
    out.push(REPORT_ID);
    out.extend_from_slice(&payload);

    let mut seen = 0usize;
    for attempt in 0..RETRIES {
        if attempt > 0 {
            thread::sleep(RETRY_GAP);
        }
        if let Err(err) = dev.write(&out) {
            diagnostics::emit_line(&format!("[aula] write 0x{marker:02X} failed: {err}"));
            return None;
        }
        let deadline = Instant::now() + REPLY_WINDOW;
        while Instant::now() < deadline {
            let mut buf = [0u8; 64];
            // A read error here is the endpoint having nothing yet, not the
            // device refusing — keep waiting out the window.
            let Ok(read) = dev.read_timeout(&mut buf, READ_SLICE_MS) else {
                continue;
            };
            if read == 0 {
                continue;
            }
            seen += 1;
            // Numbered reports arrive with the ID in front of the frame.
            let body = if buf[0] == REPORT_ID {
                &buf[1..read]
            } else {
                &buf[..read]
            };
            if body.len() < PAYLOAD_LEN || body[0] != marker {
                continue;
            }
            let frame = &body[..PAYLOAD_LEN];
            if frame[PAYLOAD_LEN - 1] != checksum(&frame[..PAYLOAD_LEN - 1]) {
                continue;
            }
            let length = frame[3] as usize;
            return Some(frame[4..(4 + length).min(PAYLOAD_LEN - 1)].to_vec());
        }
    }
    diagnostics::emit_line(&format!(
        "[aula] no reply to 0x{marker:02X} after {RETRIES} tries ({seen} unrelated report(s))"
    ));
    None
}

/// State of charge, or `None` when the keyboard behind the receiver is off.
fn battery(dev: &HidDevice) -> Option<u8> {
    let data = ask(dev, &[CMD_BATTERY, 0, 0, 0], CMD_BATTERY)?;
    // data[1] is a status byte the configurator does not surface; left alone
    // rather than guessed at as a charging flag.
    data.first().copied().filter(|p| (1..=100).contains(p))
}

fn model(dev: &HidDevice) -> Option<&'static str> {
    let data = ask(dev, &[CMD_UUID, 0x01, 0, 0, 0, 0, 0], CMD_UUID)?;
    if data.len() < 6 {
        return None;
    }
    let id = data[..6]
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | *byte as u64);
    KNOWN_MODELS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| *name)
}

/// A receiver names itself after the radio, not after what is paired with it.
fn looks_like_a_receiver(product: &str) -> bool {
    let name = product.to_ascii_lowercase();
    ["receiver", "dongle", "2.4g", "2.4 g", "wireless"]
        .iter()
        .any(|hint| name.contains(hint))
}

fn label(dev: &HidDevice, info: &DeviceInfo) -> String {
    let key = (info.vendor_id(), info.product_id());
    if let Some(name) = model_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return name;
    }
    let name = match model(dev) {
        Some(model) => model.to_string(),
        None => match info.product_string().map(str::trim).unwrap_or_default() {
            product if !product.is_empty() && !looks_like_a_receiver(product) => product.to_string(),
            _ => "Aula".to_string(),
        },
    };
    if let Ok(mut cache) = model_cache().lock() {
        cache.insert(key, name.clone());
    }
    name
}

fn cached_label(info: &DeviceInfo) -> String {
    model_cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&(info.vendor_id(), info.product_id())).cloned())
        .unwrap_or_else(|| "Aula".to_string())
}

/// Every Aula radio currently plugged in — no PID table, so a keyboard bought
/// later is found the moment it is paired.
pub fn read_all() -> Vec<DeviceReading> {
    hid::with_api(|api| {
        let mut out = Vec::new();
        let mut queried: Vec<(u16, u16, i32)> = Vec::new();
        for info in api.device_list() {
            if !AULA_VIDS.contains(&info.vendor_id()) || info.usage_page() != VENDOR_USAGE_PAGE {
                continue;
            }
            let key = (info.vendor_id(), info.product_id(), info.interface_number());
            if queried.contains(&key) {
                continue;
            }
            queried.push(key);

            let Ok(dev) = api.open_path(info.path()) else {
                continue;
            };
            let _ = dev.set_blocking_mode(false);

            let transport = hid::transport_label(info);
            match battery(&dev) {
                Some(percent) => {
                    let name = label(&dev, info);
                    out.push(
                        DeviceReading::ok(Brand::aula(), name, transport, percent, false)
                            .ranked(RANK_VENDOR),
                    );
                }
                None => out.push(DeviceReading::failed(
                    Brand::aula(),
                    cached_label(info),
                    transport,
                    "Aula receiver found but the keyboard did not answer (is it switched on?).",
                    true,
                )),
            }
        }
        out
    })
    .unwrap_or_default()
}

pub fn receiver_present() -> bool {
    hid::with_api(|api| {
        api.device_list().any(|info| {
            AULA_VIDS.contains(&info.vendor_id()) && info.usage_page() == VENDOR_USAGE_PAGE
        })
    })
    .unwrap_or(false)
}
