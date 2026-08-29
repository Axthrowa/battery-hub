//! Explicit "add a device" scan.
//!
//! The automatic readers only report a battery they can identify for certain.
//! This scan goes one step further for hardware that hides its state of charge
//! in a report nobody standardised: it samples every report twice and offers
//! the bytes that stayed put and look like a percentage. The user recognises
//! the real value, and only then is the exact location stored (see `learned`).
//!
//! Both channels are sampled. A feature report is asked for by ID, but plenty
//! of hardware — a wireless gamepad above all — answers no feature report at
//! all and publishes everything it has on its interrupt endpoint. Probing only
//! the first left those devices with nothing to offer, and the scan then
//! dropped them without a word, so a connected controller simply never
//! appeared. Nothing is dropped now: a device the scan cannot read is still
//! listed, carrying the reason it cannot.

use super::hid;
use super::hid_battery::feature_payload;
use super::hid_descriptor::{self, BatteryLayout};
use super::learned::{self, LearnedDevice, ReportSource};
use hidapi::HidDevice;
use serde::Serialize;
use std::collections::HashSet;
use std::thread;
use std::time::Duration;

const REPORT_MAX: usize = 64;
/// HID_MAX_DESCRIPTOR_SIZE.
const DESCRIPTOR_MAX: usize = 4096;
/// Report IDs worth probing — vendor reports live low.
const PROBE_REPORT_IDS: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];
/// Only the head of a vendor feature report can plausibly hold a charge.
const FEATURE_SCAN_BYTES: usize = 16;
/// An input report is laid out by its descriptor rather than by convention, and
/// the charge is appended after the controls: on a full-size controller frame
/// that puts it well past the head.
const INPUT_SCAN_BYTES: usize = REPORT_MAX;
const MAX_VALUES_PER_DEVICE: usize = 12;
/// Gap between the two samples used to reject counters and movement data.
const SAMPLE_GAP: Duration = Duration::from_millis(180);
/// A sweep opens every HID interface on the machine, so waiting on each one
/// adds up. A controller sends at 60 Hz or better; anything still silent after
/// this only reports when it is being used, and has nothing to offer a scan.
const INPUT_TIMEOUT_MS: i32 = 30;

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
    /// Feature or input — the poll has to ask the same way the scan did.
    pub source: ReportSource,
    /// Only a numbered descriptor puts the report ID in front of an interrupt
    /// frame, so this decides where the payload starts on every later read.
    pub uses_report_ids: bool,
}

/// Why the scan has nothing to offer for a device.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Blocked {
    /// Answered no feature report and sent no input report either.
    Silent,
    /// It did report — nothing that came back reads like a percentage.
    NoPercentByte,
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
    /// Set only when there is nothing to offer, and says which kind of nothing.
    pub blocked: Option<Blocked>,
    /// Whether any report at all came back, which is what separates the two
    /// reasons above.
    #[serde(skip_serializing)]
    answered: bool,
}

/// One interrupt frame, whatever report it happens to belong to.
///
/// A feature report is requested by ID; an input report simply arrives, so the
/// scan takes what the endpoint offers and records which report that was.
fn input_probe(dev: &HidDevice, uses_report_ids: bool) -> Option<(u8, Vec<u8>)> {
    let mut buf = [0u8; REPORT_MAX];
    let read = dev.read_timeout(&mut buf, INPUT_TIMEOUT_MS).ok()?;
    let end = read.min(REPORT_MAX);
    if end == 0 {
        return None;
    }
    if !uses_report_ids {
        return Some((0, buf[..end].to_vec()));
    }
    // Reading the leading ID byte as data is the `0x02 -> 2%` bug.
    (end > 1).then(|| (buf[0], buf[1..end].to_vec()))
}

fn layout_of(dev: &HidDevice) -> Option<BatteryLayout> {
    let mut raw = [0u8; DESCRIPTOR_MAX];
    let read = dev.get_report_descriptor(&mut raw).ok()?;
    let end = read.min(DESCRIPTOR_MAX);
    (end > 0).then(|| hid_descriptor::parse(&raw[..end]))
}

/// Which bytes of this report the descriptor spends on axes and buttons.
///
/// Left alone, a controller holds its sticks perfectly still and its buttons
/// released, so those bytes hold as steady across two samples as a state of
/// charge does — and a resting axis lands inside the 1..=100 window often
/// enough that the scan would offer one as a battery on hardware that has no
/// readable battery at all.
fn control_mask(layout: Option<&BatteryLayout>, report_id: u8, len: usize) -> Vec<bool> {
    match layout {
        Some(layout) => (0..len)
            .map(|offset| layout.is_control_byte(report_id, offset))
            .collect(),
        None => Vec::new(),
    }
}

/// Bytes that look like a percentage in both samples and did not move.
fn stable_percent_bytes(first: &[u8], second: &[u8], scan_bytes: usize) -> Vec<(usize, u8)> {
    first
        .iter()
        .zip(second.iter())
        .take(scan_bytes)
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
        let Some(later) = second.iter().find(|other| {
            other.path == probe.path
                && other.source == probe.source
                && other.report_id == probe.report_id
        }) else {
            continue;
        };

        let scan_bytes = match probe.source {
            ReportSource::Feature => FEATURE_SCAN_BYTES,
            ReportSource::Input => INPUT_SCAN_BYTES,
        };
        let values: Vec<CandidateValue> =
            stable_percent_bytes(&probe.payload, &later.payload, scan_bytes)
                .into_iter()
                .filter(|(offset, _)| !probe.controls.get(*offset).copied().unwrap_or(false))
                .map(|(byte_offset, percent)| CandidateValue {
                    usage_page: probe.usage_page,
                    usage: probe.usage,
                    interface: probe.interface,
                    report_id: probe.report_id,
                    byte_offset,
                    percent,
                    source: probe.source,
                    uses_report_ids: probe.uses_report_ids,
                })
                .collect();

        let added = values.iter().any(|value| {
            taught.contains(&LearnedDevice::key(
                probe.vendor_id,
                probe.product_id,
                value.usage_page,
                value.report_id,
                value.byte_offset,
                value.source,
            ))
        });

        // One entry per physical device: merge the collections behind it.
        let entry = match candidates.iter().position(|e| e.id == probe.id) {
            Some(at) => &mut candidates[at],
            None => {
                candidates.push(DeviceCandidate {
                    id: probe.id.clone(),
                    name: probe.name.clone(),
                    vendor_id: probe.vendor_id,
                    product_id: probe.product_id,
                    automatic: false,
                    added: false,
                    values: Vec::new(),
                    blocked: None,
                    answered: false,
                });
                candidates.last_mut().expect("just pushed")
            }
        };
        entry.automatic |= probe.automatic;
        entry.added |= added;
        entry.answered |= probe.answered;
        for value in values {
            if entry.values.len() >= MAX_VALUES_PER_DEVICE {
                break;
            }
            // The user picks by the number they can read off the device, so a
            // second byte holding the same value in the same collection adds a
            // chip they cannot tell apart from the first.
            let duplicate = entry
                .values
                .iter()
                .any(|kept| kept.percent == value.percent && kept.usage_page == value.usage_page);
            if !duplicate {
                entry.values.push(value);
            }
        }
    }

    // Nothing is discarded. A device the scan cannot read is still listed with
    // the reason, because a scan that silently omits the hardware someone is
    // sitting in front of reads as a scan that never saw it.
    for candidate in &mut candidates {
        if candidate.automatic || !candidate.values.is_empty() {
            continue;
        }
        candidate.blocked = Some(if candidate.answered {
            Blocked::NoPercentByte
        } else {
            Blocked::Silent
        });
    }

    candidates.sort_by(|a, b| {
        a.blocked
            .is_some()
            .cmp(&b.blocked.is_some())
            .then_with(|| a.automatic.cmp(&b.automatic))
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
    uses_report_ids: bool,
    source: ReportSource,
    report_id: u8,
    automatic: bool,
    /// A report actually came back, as opposed to a placeholder that exists
    /// only to keep the device in the list.
    answered: bool,
    /// Indexed by payload offset; empty where the descriptor said nothing.
    controls: Vec<bool>,
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

            let layout = layout_of(&dev);
            // A declared battery field means an automatic reader already has
            // it — and so does a vendor the specialized readers speak for.
            let automatic = hid::covered_by_a_reader(info.vendor_id())
                || layout.as_ref().is_some_and(BatteryLayout::has_battery);
            // No descriptor: assume numbered reports, matching the feature
            // convention, so an ID is never mistaken for a percentage.
            let uses_report_ids = layout.as_ref().map(|l| l.uses_report_ids).unwrap_or(true);

            let id = format!("{:04X}:{:04X}", info.vendor_id(), info.product_id());
            let path = info.path().to_string_lossy().into_owned();

            let mut push = |source: ReportSource,
                            report_id: u8,
                            payload: Vec<u8>,
                            answered: bool,
                            controls: Vec<bool>| {
                out.push(Probe {
                    id: id.clone(),
                    name: product.clone(),
                    path: path.clone(),
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    usage_page: info.usage_page(),
                    usage: info.usage(),
                    interface: info.interface_number(),
                    uses_report_ids,
                    source,
                    report_id,
                    automatic,
                    answered,
                    controls,
                    payload,
                });
            };

            if automatic {
                push(ReportSource::Feature, 0, Vec::new(), false, Vec::new());
                continue;
            }

            let mut answered = false;
            for report_id in PROBE_REPORT_IDS {
                let Some(payload) = feature_payload(&dev, report_id) else {
                    continue;
                };
                answered = true;
                push(ReportSource::Feature, report_id, payload, true, Vec::new());
            }

            // The channel a controller reports on, and the one this scan used
            // to skip entirely.
            if let Some((report_id, payload)) = input_probe(&dev, uses_report_ids) {
                answered = true;
                let controls = control_mask(layout.as_ref(), report_id, payload.len());
                push(ReportSource::Input, report_id, payload, true, controls);
            }

            if !answered {
                push(ReportSource::Feature, 0, Vec::new(), false, Vec::new());
            }
        }
        out
    })
    .unwrap_or_default()
}
