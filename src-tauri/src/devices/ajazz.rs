//! Ajazz 2.4G 8K (VID=0x3151 PID=0x5007) battery via vendor status report.
//!
//! 1. Open collection usage_page=0xFFFF usage=0x02 (MI_02)
//! 2. SET_FEATURE `[0x00, 0xF7, …]` — wakes 2.4 GHz telemetry
//! 3. Wait ~50–80 ms
//! 4. GET_FEATURE report `0x05` → `05 00 00 NN 01 LL …`
//!
//! `NN` is the percentage and `LL` says whether the mouse is on the link — see
//! `parse_report05`, which is also where the byte that was long mistaken for a
//! charging flag is explained.

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

/// `00 00 <percent> 01 <link> 01 02`, with the leading report ID present only
/// on some reads.
///
/// The fifth byte was read as a charging flag for a long time, and it is not
/// one — it says whether the mouse is on the 2.4 GHz link. Watched across a
/// power switch it moves within seconds: `00` while the mouse is on the air,
/// `01` the moment it goes, and back to `00` when it returns, with the level
/// refreshing on the same breath.
///
/// The mistake was reasonable, because the two look alike from one angle: a
/// cable takes the mouse off the radio, so plugging one in does set the byte —
/// but so does the mouse simply falling asleep on the desk. That is where the
/// phantom came from. The receiver keeps serving the last frame it heard from
/// a mouse that has gone quiet, and every one of those frames was being read
/// as "charging", for hours at a time.
///
/// So there is no charging indicator here at all: the one state in which the
/// mouse is charging is the one state in which it is not talking. What this
/// byte gives instead is worth more — whether the level beside it describes
/// this moment or some earlier one.
fn parse_report05(buf: &[u8]) -> Option<(u8, bool)> {
    let body = if buf.first() == Some(&0x05) {
        buf.get(1..)?
    } else {
        buf
    };
    if body.len() < 5 || body[0] != 0 || body[1] != 0 {
        return None;
    }
    let percent = body[2];
    if !(1..=100).contains(&percent) {
        return None;
    }
    let on_air = body[4] == 0;
    Some((percent, on_air))
}

fn read_aj_series_battery(dev: &HidDevice) -> Option<(u8, bool)> {
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
            if let Some(reading) = parse_report05(&buf[..n.max(8)]) {
                return Some(reading);
            }
        }
    }
    None
}

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
                if let Some((percent, on_air)) = read_aj_series_battery(&dev) {
                    let brand = Brand::classify("", product);
                    if !on_air {
                        // The receiver answered, but for a mouse that is not
                        // there: switched off, asleep, or on a cable, which
                        // takes it off the radio too. The level it is still
                        // serving belongs to whenever the mouse last spoke, so
                        // there is nothing to show — a card standing there with
                        // an hours-old number is worse than no card.
                        return DeviceReading::failed(
                            brand,
                            product,
                            "2.4 GHz",
                            "Receiver found, but the mouse is not on the 2.4 GHz link.",
                            true,
                        )
                        .of_kind(crate::devices::DeviceKind::Mouse);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_frame_carries_the_level() {
        let frame = [0x05, 0x00, 0x00, 0x5C, 0x01, 0x00, 0x01, 0x02];
        assert_eq!(parse_report05(&frame), Some((92, true)));
    }

    /// The same frame with the link byte set is what the receiver repeats after
    /// the mouse goes quiet — measured across a power switch, where it flipped
    /// within seconds of the mouse leaving and returning.
    #[test]
    fn the_link_byte_marks_a_frame_the_mouse_did_not_send() {
        let frame = [0x05, 0x00, 0x00, 0x5C, 0x01, 0x01, 0x01, 0x02];
        assert_eq!(parse_report05(&frame), Some((92, false)));
    }

    #[test]
    fn the_report_id_is_optional() {
        let with_id = [0x05, 0x00, 0x00, 0x5B, 0x01, 0x00, 0x01, 0x02];
        let without = [0x00, 0x00, 0x5B, 0x01, 0x00, 0x01, 0x02];
        assert_eq!(parse_report05(&with_id), parse_report05(&without));
    }

    #[test]
    fn nonsense_and_short_frames_are_refused() {
        assert_eq!(parse_report05(&[0x00, 0x00, 0x5C, 0x01]), None, "too short");
        assert_eq!(parse_report05(&[0x00, 0x01, 0x5C, 0x01, 0x00]), None, "bad prefix");
        assert_eq!(parse_report05(&[0x00, 0x00, 0x00, 0x01, 0x00]), None, "0%");
        assert_eq!(parse_report05(&[0x00, 0x00, 0x65, 0x01, 0x00]), None, "101%");
    }
}
