//! Explicit "add a device" scan.
//!
//! The automatic readers only report a battery they can identify for certain.
//! This scan goes one step further for hardware that hides its state of charge
//! in a vendor feature report: it samples every report twice and offers the
//! bytes that stayed put and look like a percentage. The user recognises the
//! real value, and only then is the exact location stored (see `learned`).

use super::hid;
use super::hid_descriptor;
use super::learned::{self, LearnedDevice};
use hidapi::HidDevice;
use serde::Serialize;
use std::collections::HashSet;
use std::thread;
use std::time::Duration;

const REPORT_MAX: usize = 64;
/// Report IDs worth probing — vendor reports live low.
const PROBE_REPORT_IDS: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];
/// Only the head of a report can plausibly hold a state of charge.
const SCAN_BYTES: usize = 16;
const MAX_VALUES_PER_DEVICE: usize = 8;
/// Gap between the two samples used to reject counters and movement data.
const SAMPLE_GAP: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateValue {
    pub usage_page: u16,
    /// Usage + interface pin the reading to the collection probed here, so the
    /// poll cannot drift onto a sibling collection that answers the same
    /// report with a byte that never changes.
    pub usage: u16,
    pub interface: i32,
    pub report_id: u8,
    pub byte_offset: usize,
    pub percent: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCandidate {
    pub id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// The device already reports a battery through a standard field.
    pub automatic: bool,
    /// The user has already taught this device.
    pub added: bool,
    pub values: Vec<CandidateValue>,
}

fn feature_payload(dev: &HidDevice, report_id: u8) -> Option<Vec<u8>> {
    let mut buf = [0u8; REPORT_MAX];
    buf[0] = report_id;
    let read = dev.get_feature_report(&mut buf).ok()?;
    let end = read.min(REPORT_MAX);
    (end > 1).then(|| buf[1..end].to_vec())
}

/// Bytes that look like a percentage in both samples and did not move.
fn stable_percent_bytes(first: &[u8], second: &[u8]) -> Vec<(usize, u8)> {
    first
        .iter()
        .zip(second.iter())
        .take(SCAN_BYTES)
        .enumerate()
        .filter(|(_, (a, b))| a == b && (1..=100).contains(*a))
        .map(|(offset, (value, _))| (offset, *value))
        .collect()
}

fn device_label(product: Option<&str>, vendor_id: u16, product_id: u16) -> String {
    product
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .unwrap_or_else(|| format!("HID {vendor_id:04X}:{product_id:04X}"))
}

fn is_system_battery(manufacturer: &str, product: &str) -> bool {
    let hay = format!("{manufacturer} {product}").to_ascii_lowercase();
    hay.contains("system") || hay.contains("acpi") || hay.contains("primary battery")
}

/// Probe every connected HID device for a byte the user can confirm.
///
/// Two sweeps with a single gap between them: sampling each report twice in
/// place would multiply the delay by the number of reports and turn a scan
/// into a minute of waiting.
pub fn scan() -> Vec<DeviceCandidate> {
    let taught: HashSet<String> = learned::all().into_iter().map(|d| d.id).collect();

    let first = sweep();
    thread::sleep(SAMPLE_GAP);
    let second = sweep();

    let mut candidates: Vec<DeviceCandidate> = Vec::new();
    for probe in first {
        let Some(later) = second
            .iter()
            .find(|other| other.path == probe.path && other.report_id == probe.report_id)
        else {
            continue;
        };

        let values: Vec<CandidateValue> = stable_percent_bytes(&probe.payload, &later.payload)
            .into_iter()
            .take(MAX_VALUES_PER_DEVICE)
            .map(|(byte_offset, percent)| CandidateValue {
                usage_page: probe.usage_page,
                usage: probe.usage,
                interface: probe.interface,
                report_id: probe.report_id,
                byte_offset,
                percent,
            })
            .collect();

        let added = values.iter().any(|value| {
            taught.contains(&LearnedDevice::key(
                probe.vendor_id,
                probe.product_id,
                value.usage_page,
                value.report_id,
                value.byte_offset,
            ))
        });

        // One entry per physical device: merge the collections behind it.
        match candidates.iter_mut().find(|existing| existing.id == probe.id) {
            Some(existing) => {
                existing.automatic |= probe.automatic;
                existing.added |= added;
                for value in values {
                    if existing.values.len() >= MAX_VALUES_PER_DEVICE {
                        break;
                    }
                    let duplicate = existing
                        .values
                        .iter()
                        .any(|kept| kept.percent == value.percent && kept.usage_page == value.usage_page);
                    if !duplicate {
                        existing.values.push(value);
                    }
                }
            }
            None => candidates.push(DeviceCandidate {
                id: probe.id,
                name: probe.name,
                vendor_id: probe.vendor_id,
                product_id: probe.product_id,
                automatic: probe.automatic,
                added,
                values,
            }),
        }
    }

    candidates.retain(|candidate| candidate.automatic || !candidate.values.is_empty());
    candidates.sort_by(|a, b| {
        a.automatic
            .cmp(&b.automatic)
            .then_with(|| b.values.len().cmp(&a.values.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    candidates
}

struct Probe {
    id: String,
    name: String,
    path: String,
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    interface: i32,
    report_id: u8,
    automatic: bool,
    payload: Vec<u8>,
}

/// One pass over every HID interface, reading every probe report once.
fn sweep() -> Vec<Probe> {
    hid::with_api(|api| {
        let mut out = Vec::new();
        for info in api.device_list() {
            let product = device_label(info.product_string(), info.vendor_id(), info.product_id());
            let manufacturer = info.manufacturer_string().unwrap_or("");
            if is_system_battery(manufacturer, &product) {
                continue;
            }
            let Ok(dev) = api.open_path(info.path()) else {
                continue;
            };

            // A declared battery field means an automatic reader already has
            // it — and so does a vendor the specialized readers speak for.
            let mut raw = [0u8; 4096];
            let automatic = hid::covered_by_a_reader(info.vendor_id())
                || dev
                    .get_report_descriptor(&mut raw)
                    .ok()
                    .map(|read| hid_descriptor::parse(&raw[..read.min(4096)]).has_battery())
                    .unwrap_or(false);

            let id = format!("{:04X}:{:04X}", info.vendor_id(), info.product_id());
            let path = info.path().to_string_lossy().into_owned();

            if automatic {
                out.push(Probe {
                    id: id.clone(),
                    name: product.clone(),
                    path: path.clone(),
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    usage_page: info.usage_page(),
                    usage: info.usage(),
                    interface: info.interface_number(),
                    report_id: 0,
                    automatic,
                    payload: Vec::new(),
                });
                continue;
            }

            for report_id in PROBE_REPORT_IDS {
                let Some(payload) = feature_payload(&dev, report_id) else {
                    continue;
                };
                out.push(Probe {
                    id: id.clone(),
                    name: product.clone(),
                    path: path.clone(),
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    usage_page: info.usage_page(),
                    usage: info.usage(),
                    interface: info.interface_number(),
                    report_id,
                    automatic,
                    payload,
                });
            }
        }
        out
    })
    .unwrap_or_default()
}
