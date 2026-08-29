//! Minimal HID report-descriptor parser — pure logic, no hidapi, unit tested.
//!
//! Windows exposes the raw descriptor through `HidDevice::get_report_descriptor`,
//! which lets the generic battery reader address the exact bits holding the state
//! of charge instead of scanning a report for "a byte that looks like a percent".
//! That guess reads a report ID of `0x02` as `2%`, so the descriptor path is
//! always tried first and the scan only survives as a last resort.

/// Generic Device Controls — `Battery Strength` lives here (most wireless HID).
pub const PAGE_GENERIC_DEVICE: u16 = 0x06;
/// Battery System — the HID Power Device page.
pub const PAGE_BATTERY_SYSTEM: u16 = 0x85;

const USAGE_BATTERY_STRENGTH: u16 = 0x20; // page 0x06
const USAGE_CHARGING: u16 = 0x44; // page 0x85
const USAGE_ABSOLUTE_STATE_OF_CHARGE: u16 = 0x65; // page 0x85
const USAGE_REMAINING_CAPACITY: u16 = 0x66; // page 0x85

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Input,
    Feature,
}

/// One addressable field inside a report, in bits relative to the payload
/// (i.e. *after* the leading report-ID byte hidapi hands back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryField {
    pub report_id: u8,
    pub kind: ReportKind,
    pub bit_offset: u32,
    pub bit_size: u32,
    pub logical_min: i32,
    pub logical_max: i32,
}

impl BatteryField {
    pub fn same_report(&self, other: &BatteryField) -> bool {
        self.kind == other.kind && self.report_id == other.report_id
    }
}

/// Payload bytes an Input report spends on axes and buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRange {
    pub report_id: u8,
    pub first_byte: usize,
    /// Inclusive — a field ending mid-byte still owns the whole byte.
    pub last_byte: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatteryLayout {
    /// Descriptor declares Report IDs → hidapi's `read()` prefixes the ID byte.
    pub uses_report_ids: bool,
    pub charge: Option<BatteryField>,
    pub charging: Option<BatteryField>,
    /// Where the controls sit in each Input report. A gamepad left alone holds
    /// its sticks perfectly still, so those bytes pass a "held steady and
    /// reads like a percentage" test as convincingly as a real state of
    /// charge does — and the scan would offer a resting axis as a battery.
    pub control_bytes: Vec<ControlRange>,
}

impl BatteryLayout {
    pub fn has_battery(&self) -> bool {
        self.charge.is_some()
    }

    /// Whether this Input payload byte is spoken for by a declared control.
    pub fn is_control_byte(&self, report_id: u8, offset: usize) -> bool {
        self.control_bytes.iter().any(|range| {
            range.report_id == report_id
                && offset >= range.first_byte
                && offset <= range.last_byte
        })
    }
}

#[derive(Clone, Copy)]
struct GlobalState {
    usage_page: u16,
    logical_min: i32,
    logical_max: i32,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

impl GlobalState {
    fn new() -> Self {
        Self {
            usage_page: 0,
            logical_min: 0,
            logical_max: 0,
            report_size: 0,
            report_count: 0,
            report_id: 0,
        }
    }
}

/// Walk the descriptor and locate the state-of-charge / charging fields.
///
/// Malformed or truncated input never panics — parsing simply stops.
pub fn parse(desc: &[u8]) -> BatteryLayout {
    let mut out = BatteryLayout::default();
    let mut g = GlobalState::new();
    let mut stack: Vec<GlobalState> = Vec::new();
    let mut usages: Vec<u32> = Vec::new();
    let mut usage_min: Option<u32> = None;
    let mut usage_max: Option<u32> = None;
    // Input and Feature reports have independent bit cursors, one per report ID.
    let mut cursors: Vec<((ReportKind, u8), u32)> = Vec::new();

    let mut i = 0usize;
    while i < desc.len() {
        let prefix = desc[i];

        // Long item: [0xFE][data size][tag][data…] — no battery data, skip it.
        if prefix == 0xFE {
            let size = *desc.get(i + 1).unwrap_or(&0) as usize;
            i = i.saturating_add(3 + size);
            continue;
        }

        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let start = i + 1;
        let Some(end) = start.checked_add(size).filter(|e| *e <= desc.len()) else {
            break; // truncated item
        };
        let data = &desc[start..end];
        let uval = le_u32(data);
        let btype = (prefix >> 2) & 0x03;
        let btag = prefix >> 4;

        match (btype, btag) {
            // ---- Global items ----
            (1, 0x0) => g.usage_page = uval as u16,
            (1, 0x1) => g.logical_min = le_i32(data),
            // Logical Maximum is signed only when the minimum is negative.
            (1, 0x2) => {
                g.logical_max = if g.logical_min < 0 {
                    le_i32(data)
                } else {
                    uval as i32
                }
            }
            (1, 0x7) => g.report_size = uval,
            (1, 0x8) => {
                g.report_id = uval as u8;
                out.uses_report_ids = true;
            }
            (1, 0x9) => g.report_count = uval,
            (1, 0xA) => stack.push(g),
            (1, 0xB) => {
                if let Some(prev) = stack.pop() {
                    g = prev;
                }
            }

            // ---- Local items ----
            (2, 0x0) => usages.push(expand_usage(uval, size, g.usage_page)),
            (2, 0x1) => usage_min = Some(expand_usage(uval, size, g.usage_page)),
            (2, 0x2) => usage_max = Some(expand_usage(uval, size, g.usage_page)),

            // ---- Main items: Input / Feature carry readable data ----
            (0, 0x8) | (0, 0xB) => {
                let kind = if btag == 0x8 {
                    ReportKind::Input
                } else {
                    ReportKind::Feature
                };
                let base = cursor(&cursors, kind, g.report_id);
                // Constant (padding) fields still occupy bits but carry no usage.
                if uval & 0x01 == 0 {
                    collect(&mut out, &g, kind, base, &usages, usage_min, usage_max);
                }
                let width = g.report_size.saturating_mul(g.report_count);
                if kind == ReportKind::Input && uval & 0x01 == 0 {
                    note_controls(&mut out, &g, base, width);
                }
                bump(&mut cursors, kind, g.report_id, width);
                usages.clear();
                usage_min = None;
                usage_max = None;
            }
            // Output reports have their own offset space we never read; every
            // other Main item (Output / Collection / End Collection) just
            // resets the local item state.
            (0, _) => {
                usages.clear();
                usage_min = None;
                usage_max = None;
            }
            _ => {}
        }

        i = end;
    }

    out
}

/// Generic Desktop and Button — where a controller declares its own inputs.
const PAGE_GENERIC_DESKTOP: u16 = 0x01;
const PAGE_BUTTON: u16 = 0x09;

/// Remember the bytes an Input field on a control page occupies.
///
/// A battery never hides in one of these: firmware that reports a charge does
/// it on the Generic Device Controls page, the Battery System page, or a
/// vendor page of its own. Anything declared as an axis or a button is a
/// control, and a control is exactly what the byte scan cannot tell apart
/// from a charge while the device sits untouched.
fn note_controls(out: &mut BatteryLayout, g: &GlobalState, base_bits: u32, width_bits: u32) {
    if width_bits == 0 || !matches!(g.usage_page, PAGE_GENERIC_DESKTOP | PAGE_BUTTON) {
        return;
    }
    out.control_bytes.push(ControlRange {
        report_id: g.report_id,
        first_byte: (base_bits / 8) as usize,
        last_byte: (base_bits.saturating_add(width_bits - 1) / 8) as usize,
    });
}

fn cursor(cursors: &[((ReportKind, u8), u32)], kind: ReportKind, id: u8) -> u32 {
    cursors
        .iter()
        .find(|(key, _)| *key == (kind, id))
        .map(|(_, bits)| *bits)
        .unwrap_or(0)
}

fn bump(cursors: &mut Vec<((ReportKind, u8), u32)>, kind: ReportKind, id: u8, width: u32) {
    if let Some((_, bits)) = cursors.iter_mut().find(|(key, _)| *key == (kind, id)) {
        *bits = bits.saturating_add(width);
    } else {
        cursors.push(((kind, id), width));
    }
}

fn collect(
    out: &mut BatteryLayout,
    g: &GlobalState,
    kind: ReportKind,
    base_bits: u32,
    usages: &[u32],
    usage_min: Option<u32>,
    usage_max: Option<u32>,
) {
    if g.report_size == 0 || g.report_count == 0 {
        return;
    }
    for index in 0..g.report_count {
        let Some(usage) = usage_at(usages, usage_min, usage_max, index) else {
            continue;
        };
        let page = (usage >> 16) as u16;
        let id = (usage & 0xFFFF) as u16;
        let field = BatteryField {
            report_id: g.report_id,
            kind,
            bit_offset: base_bits.saturating_add(index.saturating_mul(g.report_size)),
            bit_size: g.report_size,
            logical_min: g.logical_min,
            logical_max: g.logical_max,
        };
        match (page, id) {
            (PAGE_GENERIC_DEVICE, USAGE_BATTERY_STRENGTH)
            | (PAGE_BATTERY_SYSTEM, USAGE_ABSOLUTE_STATE_OF_CHARGE)
            | (PAGE_BATTERY_SYSTEM, USAGE_REMAINING_CAPACITY) => {
                // First declaration wins — descriptors list the primary field first.
                if out.charge.is_none() {
                    out.charge = Some(field);
                }
            }
            (PAGE_BATTERY_SYSTEM, USAGE_CHARGING) if out.charging.is_none() => {
                out.charging = Some(field);
            }
            _ => {}
        }
    }
}

fn usage_at(usages: &[u32], min: Option<u32>, max: Option<u32>, index: u32) -> Option<u32> {
    if !usages.is_empty() {
        // Fewer usages than fields: the last one repeats for the remainder.
        let at = (index as usize).min(usages.len() - 1);
        return usages.get(at).copied();
    }
    let min = min?;
    let max = max.unwrap_or(min);
    let usage = min.checked_add(index)?;
    (usage <= max).then_some(usage)
}

fn expand_usage(value: u32, size: usize, page: u16) -> u32 {
    // A 4-byte usage carries its own page in the high half.
    if size == 4 {
        value
    } else {
        ((page as u32) << 16) | (value & 0xFFFF)
    }
}

fn le_u32(data: &[u8]) -> u32 {
    data.iter()
        .take(4)
        .enumerate()
        .fold(0u32, |acc, (i, b)| acc | ((*b as u32) << (8 * i)))
}

fn le_i32(data: &[u8]) -> i32 {
    match data.len() {
        0 => 0,
        1 => data[0] as i8 as i32,
        2 => i16::from_le_bytes([data[0], data[1]]) as i32,
        _ => le_u32(data) as i32,
    }
}

/// Little-endian bit extraction, as HID lays fields out inside a report.
pub fn extract_bits(payload: &[u8], bit_offset: u32, bit_size: u32) -> Option<u32> {
    if bit_size == 0 || bit_size > 32 {
        return None;
    }
    let end = bit_offset.checked_add(bit_size)?;
    if end as usize > payload.len().saturating_mul(8) {
        return None;
    }
    let mut value = 0u32;
    for bit in 0..bit_size {
        let at = bit_offset + bit;
        let byte = payload[(at / 8) as usize];
        if byte >> (at % 8) & 1 == 1 {
            value |= 1 << bit;
        }
    }
    Some(value)
}

/// Raw field value, sign-extended when the descriptor declares a signed range.
pub fn field_value(payload: &[u8], field: &BatteryField) -> Option<i32> {
    let raw = extract_bits(payload, field.bit_offset, field.bit_size)?;
    if field.logical_min < 0 && field.bit_size < 32 {
        let sign_bit = 1u32 << (field.bit_size - 1);
        if raw & sign_bit != 0 {
            let span = 1i64 << field.bit_size;
            return Some((raw as i64 - span) as i32);
        }
    }
    Some(raw as i32)
}

/// Scale a raw value onto 0–100 using the declared logical range, so a device
/// reporting 0–255 (or 0–10 bars) still yields a real percentage.
pub fn to_percent(value: i32, field: &BatteryField) -> Option<u8> {
    let (min, max) = (field.logical_min, field.logical_max);
    if max > min {
        let clamped = value.clamp(min, max) as i64;
        let span = (max - min) as i64;
        let percent = (clamped - min as i64) * 100 / span;
        return Some(percent.clamp(0, 100) as u8);
    }
    // No usable range declared — accept the value only if it already reads as a percent.
    (0..=100).contains(&value).then_some(value as u8)
}

/// Last-resort scan for devices whose descriptor declares no battery field.
///
/// `skip_report_id` must say whether `data` still carries hidapi's leading
/// report-ID byte; treating that byte as data is exactly the `0x02 → 2%` bug
/// the descriptor path above exists to avoid.
pub fn plausible_percent(data: &[u8], skip_report_id: bool) -> Option<u8> {
    let payload = if skip_report_id {
        data.get(1..)?
    } else {
        data
    };
    payload
        .iter()
        .take(8)
        .copied()
        .find(|byte| (1..=100).contains(byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usage Page (Generic Device Controls), Usage (Battery Strength),
    /// Logical 0–100, Report ID 5, one 8-bit Feature field.
    const BATTERY_STRENGTH_FEATURE: &[u8] = &[
        0x05, 0x06, // Usage Page (Generic Device Controls)
        0x09, 0x20, // Usage (Battery Strength)
        0xA1, 0x01, // Collection (Application)
        0x85, 0x05, //   Report ID (5)
        0x09, 0x20, //   Usage (Battery Strength)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x64, //   Logical Maximum (100)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0xB1, 0x02, //   Feature (Data,Var,Abs)
        0xC0, //       End Collection
    ];

    /// Battery System page: 8 padding bits, then Charging as a single bit,
    /// then AbsoluteStateOfCharge as a byte — all in one Input report.
    const BATTERY_SYSTEM_INPUT: &[u8] = &[
        0x05, 0x85, // Usage Page (Battery System)
        0x09, 0x24, // Usage (SMB Battery Mode)
        0xA1, 0x01, // Collection (Application)
        0x85, 0x01, //   Report ID (1)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x03, //   Input (Cnst,Var,Abs)  <- 8 bits of padding
        0x09, 0x44, //   Usage (Charging)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x01, //   Logical Maximum (1)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0x75, 0x07, //   Report Size (7)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x03, //   Input (Cnst,Var,Abs)  <- 7 bits of padding
        0x09, 0x65, //   Usage (AbsoluteStateOfCharge)
        0x25, 0x64, //   Logical Maximum (100)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data,Var,Abs)
        0xC0, //       End Collection
    ];

    /// No Report ID item anywhere — hidapi hands back raw payloads.
    const UNNUMBERED: &[u8] = &[
        0x05, 0x06, // Usage Page (Generic Device Controls)
        0x09, 0x20, // Usage (Battery Strength)
        0xA1, 0x01, // Collection (Application)
        0x09, 0x20, //   Usage (Battery Strength)
        0x15, 0x00, //   Logical Minimum (0)
        0x26, 0xFF, 0x00, // Logical Maximum (255)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0xB1, 0x02, //   Feature (Data,Var,Abs)
        0xC0, //       End Collection
    ];

    /// A controller: 16 buttons, four 8-bit axes, then a vendor byte. Nothing
    /// on a battery page, and the layout an XInput-style pad actually sends.
    const GAMEPAD_INPUT: &[u8] = &[
        0x05, 0x01, // Usage Page (Generic Desktop)
        0x09, 0x05, // Usage (Gamepad)
        0xA1, 0x01, // Collection (Application)
        0x05, 0x09, //   Usage Page (Button)
        0x19, 0x01, //   Usage Minimum (1)
        0x29, 0x10, //   Usage Maximum (16)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x01, //   Logical Maximum (1)
        0x75, 0x01, //   Report Size (1)
        0x95, 0x10, //   Report Count (16)
        0x81, 0x02, //   Input (Data,Var,Abs)   <- bytes 0-1
        0x05, 0x01, //   Usage Page (Generic Desktop)
        0x09, 0x30, //   Usage (X)
        0x09, 0x31, //   Usage (Y)
        0x09, 0x32, //   Usage (Z)
        0x09, 0x35, //   Usage (Rz)
        0x15, 0x00, //   Logical Minimum (0)
        0x26, 0xFF, 0x00, // Logical Maximum (255)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x04, //   Report Count (4)
        0x81, 0x02, //   Input (Data,Var,Abs)   <- bytes 2-5
        0x06, 0x00, 0xFF, // Usage Page (Vendor Defined FF00)
        0x09, 0x20, //   Usage (vendor 0x20)
        0x25, 0x64, //   Logical Maximum (100)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x81, 0x02, //   Input (Data,Var,Abs)   <- byte 6
        0xC0, //       End Collection
    ];

    #[test]
    fn finds_battery_strength_feature_field() {
        let layout = parse(BATTERY_STRENGTH_FEATURE);
        let charge = layout.charge.expect("battery strength field");
        assert!(layout.uses_report_ids);
        assert_eq!(charge.report_id, 5);
        assert_eq!(charge.kind, ReportKind::Feature);
        assert_eq!(charge.bit_offset, 0);
        assert_eq!(charge.bit_size, 8);
        assert_eq!((charge.logical_min, charge.logical_max), (0, 100));
        assert!(layout.charging.is_none());
    }

    #[test]
    fn accounts_for_padding_before_battery_fields() {
        let layout = parse(BATTERY_SYSTEM_INPUT);
        let charge = layout.charge.expect("state of charge field");
        let charging = layout.charging.expect("charging field");
        // 8 padding bits, then the charging bit at 8, 7 more padding, SOC at 16.
        assert_eq!(charging.bit_offset, 8);
        assert_eq!(charging.bit_size, 1);
        assert_eq!(charge.bit_offset, 16);
        assert_eq!(charge.kind, ReportKind::Input);
        assert_eq!(charge.report_id, 1);

        // Payload: [padding][charging=1][SOC=77]
        let payload = [0x00u8, 0x01, 77];
        let value = field_value(&payload, &charge).unwrap();
        assert_eq!(to_percent(value, &charge), Some(77));
        assert_eq!(field_value(&payload, &charging), Some(1));
    }

    #[test]
    fn detects_descriptors_without_report_ids() {
        let layout = parse(UNNUMBERED);
        let charge = layout.charge.expect("battery strength field");
        assert!(!layout.uses_report_ids);
        assert_eq!(charge.report_id, 0);
        assert_eq!(charge.logical_max, 255);
        // 0–255 range must scale onto 0–100, not be taken literally.
        assert_eq!(to_percent(field_value(&[128], &charge).unwrap(), &charge), Some(50));
        assert_eq!(to_percent(field_value(&[255], &charge).unwrap(), &charge), Some(100));
    }

    #[test]
    fn ignores_non_battery_usages() {
        // A plain mouse: buttons + X/Y, no battery anywhere.
        let mouse: &[u8] = &[
            0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09, 0x19, 0x01,
            0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01,
            0x75, 0x05, 0x81, 0x03, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7F,
            0x75, 0x08, 0x95, 0x02, 0x81, 0x06, 0xC0, 0xC0,
        ];
        let layout = parse(mouse);
        assert!(!layout.has_battery());
        assert!(layout.charging.is_none());
    }

    #[test]
    fn survives_truncated_and_garbage_descriptors() {
        for cut in 0..BATTERY_SYSTEM_INPUT.len() {
            let _ = parse(&BATTERY_SYSTEM_INPUT[..cut]);
        }
        for cut in 0..BATTERY_STRENGTH_FEATURE.len() {
            let _ = parse(&BATTERY_STRENGTH_FEATURE[cut..]);
        }
        let _ = parse(&[0xFE, 0xFF, 0x01]); // long item claiming more data than exists
        let _ = parse(&[0xFF; 64]);
        let _ = parse(&[]);
    }

    #[test]
    fn extract_bits_respects_payload_bounds() {
        assert_eq!(extract_bits(&[0b1010_0000], 5, 3), Some(0b101));
        assert_eq!(extract_bits(&[0xFF, 0x01], 8, 8), Some(1));
        assert_eq!(extract_bits(&[0xFF], 4, 8), None); // runs past the payload
        assert_eq!(extract_bits(&[0xFF], 0, 0), None);
    }

    #[test]
    fn percent_scan_skips_the_report_id_byte() {
        // Report ID 2, then the real state of charge (88%).
        let report = [0x02u8, 88, 0x00];
        assert_eq!(plausible_percent(&report, true), Some(88));
        // The bug this replaces: the report ID itself read as "2%".
        assert_eq!(plausible_percent(&report, false), Some(2));
        assert_eq!(plausible_percent(&[0x02], true), None);
        assert_eq!(plausible_percent(&[], true), None);
    }

    /// The scan offers bytes that held still between two samples, and a pad
    /// left alone holds every axis still. Marking the controls is what keeps a
    /// resting stick from being offered as a state of charge.
    #[test]
    fn a_gamepad_marks_its_axes_and_buttons_as_controls() {
        let layout = parse(GAMEPAD_INPUT);
        assert!(!layout.has_battery());
        for offset in 0..=5 {
            assert!(
                layout.is_control_byte(0, offset),
                "byte {offset} belongs to a button or an axis"
            );
        }
        // The vendor byte is where a charge would actually be published.
        assert!(!layout.is_control_byte(0, 6));
    }

    /// Padding is constant, never a reading, and never claimed as a control:
    /// the battery byte after it has to stay offerable.
    #[test]
    fn declared_battery_bytes_are_not_controls() {
        let layout = parse(BATTERY_SYSTEM_INPUT);
        assert!(layout.has_battery());
        assert!(layout.control_bytes.is_empty());
    }
}
