//! Multi-brand battery hub — specialized readers + generic Windows/BLE/HID discovery.

mod ajazz;
mod aula;
mod brand;
pub mod ble_gatt;
pub mod diagnostics;
pub mod discover;
pub mod hid;
mod hid_battery;
mod hid_descriptor;
mod kind;
pub mod learned;
mod logitech;
mod razer;
pub mod soundcore;
mod windows_battery;

pub use brand::Brand;
pub use kind::DeviceKind;

use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What a reading is worth when two sources describe the same product.
///
/// Several readers can reach the same device at once — the vendor's own frame,
/// the battery field in its descriptor, a byte the user pointed at, a value
/// Windows cached when it last paired. They do not deserve equal weight, and
/// picking by transport alone cannot separate two sources that share a radio.
pub const RANK_GENERIC: u8 = 10;
pub const RANK_TAUGHT: u8 = 20;
pub const RANK_DESCRIPTOR: u8 = 30;
pub const RANK_VENDOR: u8 = 40;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceReading {
    pub brand: Brand,
    pub brand_label: String,
    pub ok: bool,
    pub percent: Option<u8>,
    pub charging: bool,
    pub transport: String,
    pub product: String,
    pub error: Option<String>,
    /// Dongle / receiver / paired radio is present even if SOC could not be read.
    pub present: bool,
    /// Read from a byte the user confirmed by sight (see `learned`).
    pub taught: bool,
    /// A taught byte that has stopped looking like a reading: same report, poll
    /// after poll, for hours. Shown, but not as a measurement.
    pub unverified: bool,
    /// Which kind of source produced this, one of the `RANK_*` constants.
    pub rank: u8,
    /// USB ids of the hardware behind the reading, `0` where a reader cannot
    /// say (Bluetooth names, COM ports). Two sources describing the same ids
    /// are the same device however differently they spell its name.
    pub vendor_id: u16,
    pub product_id: u16,
    /// Keyboard, mouse, headset — what to draw when nobody has given the
    /// device artwork and its brand has no logo.
    pub kind: DeviceKind,
    /// On the cable with nothing left to take. Devices keep reporting that they
    /// are charging while they sit at full, so "charging" alone would leave a
    /// finished device looking like it is still filling up.
    pub full: bool,
    pub updated_at_ms: u64,
}

impl DeviceReading {
    pub fn ok(
        brand: Brand,
        product: impl Into<String>,
        transport: impl Into<String>,
        percent: u8,
        charging: bool,
    ) -> Self {
        let brand_label = brand.label();
        Self {
            brand,
            brand_label,
            ok: true,
            percent: Some(percent.min(100)),
            charging,
            full: charging && percent >= 100,
            transport: transport.into(),
            product: product.into(),
            error: None,
            present: true,
            taught: false,
            unverified: false,
            rank: RANK_GENERIC,
            vendor_id: 0,
            product_id: 0,
            kind: DeviceKind::Device,
            updated_at_ms: now_ms(),
        }
    }

    /// Raise a reading above the generic default — see the `RANK_*` constants.
    pub fn ranked(mut self, rank: u8) -> Self {
        self.rank = rank;
        self
    }

    /// Name the hardware this was read from, so sources can be matched by ids
    /// rather than by how each of them happens to spell the product.
    pub fn measured_on(mut self, vendor_id: u16, product_id: u16) -> Self {
        self.vendor_id = vendor_id;
        self.product_id = product_id;
        self
    }

    /// When the value was actually measured, for readings served from a cache.
    pub fn measured_at(mut self, at_ms: u64) -> Self {
        self.updated_at_ms = at_ms;
        self
    }

    /// What the device is, for the placeholder artwork.
    pub fn of_kind(mut self, kind: DeviceKind) -> Self {
        self.kind = kind;
        self
    }

    /// A reading from a location the user taught. It outranks what Windows
    /// cached and what a byte scan guessed, but not a vendor's own frame: a
    /// confirmed byte is still a byte someone recognised by sight.
    pub fn taught(
        brand: Brand,
        product: impl Into<String>,
        transport: impl Into<String>,
        percent: u8,
        verified: bool,
    ) -> Self {
        Self {
            taught: true,
            unverified: !verified,
            rank: RANK_TAUGHT,
            ..Self::ok(brand, product, transport, percent, false)
        }
    }

    pub fn failed(
        brand: Brand,
        product: impl Into<String>,
        transport: impl Into<String>,
        error: impl Into<String>,
        present: bool,
    ) -> Self {
        let brand_label = brand.label();
        Self {
            brand,
            brand_label,
            ok: false,
            percent: None,
            charging: false,
            full: false,
            transport: transport.into(),
            product: product.into(),
            error: Some(error.into()),
            present,
            taught: false,
            unverified: false,
            rank: RANK_GENERIC,
            vendor_id: 0,
            product_id: 0,
            kind: DeviceKind::Device,
            updated_at_ms: now_ms(),
        }
    }

    /// Tray / tooltip line, e.g. `Razer: %80`.
    pub fn tray_line(&self) -> String {
        let label = if self.product.trim().is_empty() {
            self.brand_label.clone()
        } else {
            // Short product for tray: first 18 chars
            let p = self.product.trim();
            if p.chars().count() > 18 {
                format!("{}…", p.chars().take(16).collect::<String>())
            } else {
                p.to_string()
            }
        };
        match (self.ok, self.percent) {
            (true, Some(p)) => format!("{label}: %{p}"),
            _ if self.present => format!("{label}: —"),
            _ => format!("{label}: offline"),
        }
    }
}

fn normalize_name(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn names_overlap(a: &str, b: &str) -> bool {
    let na = normalize_name(a);
    let nb = normalize_name(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na == nb || (na.len() >= 6 && nb.contains(&na)) || (nb.len() >= 6 && na.contains(&nb))
}

/// Backward-compatible single-device shape (primary = first online).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryReading {
    pub ok: bool,
    pub percent: Option<u8>,
    pub charging: bool,
    pub transport: String,
    pub product: String,
    pub error: Option<String>,
    pub dongle_present: bool,
    pub updated_at_ms: u64,
}

impl From<&DeviceReading> for BatteryReading {
    fn from(d: &DeviceReading) -> Self {
        Self {
            ok: d.ok,
            percent: d.percent,
            charging: d.charging,
            transport: d.transport.clone(),
            product: d.product.clone(),
            error: d.error.clone(),
            dongle_present: d.present,
            updated_at_ms: d.updated_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSnapshot {
    pub devices: Vec<DeviceReading>,
    pub online: Vec<DeviceReading>,
    pub updated_at_ms: u64,
    pub primary: BatteryReading,
}

/// Only report per-reader timings when a poll is slow enough to matter.
const SLOW_POLL_SECONDS: f32 = 3.0;

type Timings = Arc<Mutex<Vec<(&'static str, f32)>>>;

fn record(timings: &Timings, reader: &'static str, started: Instant) {
    if let Ok(mut list) = timings.lock() {
        list.push((reader, started.elapsed().as_secs_f32()));
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push_unique(out: &mut Vec<DeviceReading>, device: DeviceReading) {
    if !device.ok && !device.present {
        return;
    }
    if let Some(existing) = out.iter_mut().find(|d| names_overlap(&d.product, &device.product)) {
        // Prefer a successful SOC reading over presence-only / weaker sources.
        let replace = match (existing.ok, device.ok) {
            (false, true) => true,
            (true, true) if existing.unverified != device.unverified => {
                // A byte the evidence has turned against loses to anything that
                // actually asked the hardware.
                !device.unverified
            }
            (true, true) if existing.rank != device.rank => device.rank > existing.rank,
            (true, true) => {
                // Same kind of source: prefer a dedicated radio over Bluetooth.
                let ex_spec = is_specialized_transport(&existing.transport);
                let new_spec = is_specialized_transport(&device.transport);
                new_spec && !ex_spec
            }
            _ => false,
        };
        if replace {
            *existing = device;
        }
        return;
    }
    out.push(device);
}

/// A taught byte is a stand-in for a reader that did not exist yet. Once one
/// does and it answers for the same hardware, drop the stand-in.
///
/// Matching is on USB ids, not on names: the two sources rarely spell a device
/// alike — a receiver calls itself "2.4G Wireless Receiver" while the person
/// who taught it typed the model on the box — so name matching leaves both on
/// screen and the percentage appears to flip between them from poll to poll.
fn drop_superseded_taught(readings: &mut Vec<DeviceReading>) {
    let answered: Vec<(u16, u16)> = readings
        .iter()
        .filter(|d| d.ok && !d.taught && d.vendor_id != 0)
        .map(|d| (d.vendor_id, d.product_id))
        .collect();
    readings.retain(|d| {
        !(d.taught && d.vendor_id != 0 && answered.contains(&(d.vendor_id, d.product_id)))
    });
}

fn is_specialized_transport(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    t.contains("2.4") || t.contains("hid++") || (t.contains("usb") && !t.contains("bluetooth"))
}

/// Poll specialized brands + generic Windows/BLE/HID discovery, then merge.
pub fn read_all() -> DeviceSnapshot {
    let poll_started = Instant::now();
    let timings: Timings = Arc::new(Mutex::new(Vec::new()));
    let (tx_r, rx_r) = std::sync::mpsc::channel();
    let (tx_l, rx_l) = std::sync::mpsc::channel();
    let (tx_a, rx_a) = std::sync::mpsc::channel();
    let (tx_s, rx_s) = std::sync::mpsc::channel();
    let (tx_w, rx_w) = std::sync::mpsc::channel();
    let (tx_g, rx_g) = std::sync::mpsc::channel();
    let (tx_h, rx_h) = std::sync::mpsc::channel();
    let (tx_u, rx_u) = std::sync::mpsc::channel();
    let (tx_n, rx_n) = std::sync::mpsc::channel();

    let timings_razer = timings.clone();
    let h_r = thread::spawn(move || {
        let started = Instant::now();
        let value = razer::read();
        record(&timings_razer, "razer", started);
        let _ = tx_r.send(value);
    });
    let timings_logitech = timings.clone();
    let h_l = thread::spawn(move || {
        let started = Instant::now();
        let value = logitech::read();
        record(&timings_logitech, "logitech", started);
        let _ = tx_l.send(value);
    });
    let timings_ajazz = timings.clone();
    let h_a = thread::spawn(move || {
        let started = Instant::now();
        let value = ajazz::read();
        record(&timings_ajazz, "ajazz", started);
        let _ = tx_a.send(value);
    });
    let timings_soundcore = timings.clone();
    let h_s = thread::spawn(move || {
        let started = Instant::now();
        let value = soundcore::read();
        record(&timings_soundcore, "soundcore", started);
        let _ = tx_s.send(value);
    });
    let timings_windows = timings.clone();
    let h_w = thread::spawn(move || {
        let started = Instant::now();
        let value = windows_battery::read_all();
        record(&timings_windows, "windows", started);
        let _ = tx_w.send(value);
    });
    let timings_ble = timings.clone();
    let h_g = thread::spawn(move || {
        let started = Instant::now();
        let value = ble_gatt::read_all_devices();
        record(&timings_ble, "ble", started);
        let _ = tx_g.send(value);
    });
    let timings_hid = timings.clone();
    let h_h = thread::spawn(move || {
        let started = Instant::now();
        let value = hid_battery::read_all();
        record(&timings_hid, "hid", started);
        let _ = tx_h.send(value);
    });

    let timings_aula = timings.clone();
    let h_n = thread::spawn(move || {
        let started = Instant::now();
        let value = aula::read_all();
        record(&timings_aula, "aula", started);
        let _ = tx_n.send(value);
    });
    let timings_learned = timings.clone();
    let h_u = thread::spawn(move || {
        let started = Instant::now();
        let value = learned::read_all();
        record(&timings_learned, "learned", started);
        let _ = tx_u.send(value);
    });

    let mut merged = Vec::new();

    if let Ok(d) = rx_r.recv() {
        push_unique(&mut merged, d);
    }
    if let Ok(d) = rx_l.recv() {
        push_unique(&mut merged, d);
    }
    if let Ok(d) = rx_a.recv() {
        push_unique(&mut merged, d);
    }
    if let Ok(d) = rx_s.recv() {
        push_unique(&mut merged, d);
    }
    if let Ok(list) = rx_w.recv() {
        for d in list {
            push_unique(&mut merged, d);
        }
    }
    if let Ok(list) = rx_g.recv() {
        for d in list {
            push_unique(&mut merged, d);
        }
    }
    if let Ok(list) = rx_h.recv() {
        for d in list {
            push_unique(&mut merged, d);
        }
    }
    if let Ok(list) = rx_n.recv() {
        for d in list {
            push_unique(&mut merged, d);
        }
    }
    if let Ok(list) = rx_u.recv() {
        for d in list {
            push_unique(&mut merged, d);
        }
    }

    let _ = h_r.join();
    let _ = h_l.join();
    let _ = h_a.join();
    let _ = h_s.join();
    let _ = h_w.join();
    let _ = h_g.join();
    let _ = h_h.join();
    let _ = h_n.join();
    let _ = h_u.join();

    drop_superseded_taught(&mut merged);

    // Stable-ish order: OK first, then by brand label.
    merged.sort_by(|a, b| {
        b.ok.cmp(&a.ok)
            .then_with(|| a.brand_label.cmp(&b.brand_label))
            .then_with(|| a.product.cmp(&b.product))
    });

    let elapsed = poll_started.elapsed().as_secs_f32();
    if elapsed > SLOW_POLL_SECONDS {
        let mut list = timings.lock().map(|l| l.clone()).unwrap_or_default();
        list.sort_by(|a, b| b.1.total_cmp(&a.1));
        let detail = list
            .iter()
            .map(|(reader, secs)| format!("{reader} {secs:.1}s"))
            .collect::<Vec<_>>()
            .join(", ");
        diagnostics::emit_line(&format!("[timing] slow poll {elapsed:.1}s — {detail}"));
    }

    let online: Vec<DeviceReading> = merged.iter().filter(|d| d.ok).cloned().collect();
    let primary = online
        .first()
        .or_else(|| merged.first())
        .map(BatteryReading::from)
        .unwrap_or_else(|| BatteryReading {
            ok: false,
            percent: None,
            charging: false,
            transport: String::new(),
            product: "Battery Hub".into(),
            error: Some("No devices".into()),
            dongle_present: false,
            updated_at_ms: now_ms(),
        });

    DeviceSnapshot {
        devices: merged,
        online,
        updated_at_ms: now_ms(),
        primary,
    }
}

pub fn any_radio_present() -> bool {
    razer::dongle_present()
        || logitech::receiver_present()
        || ajazz::receiver_present()
        || aula::receiver_present()
}

/// Primary device across every brand — not just the Razer link.
pub fn read_battery() -> BatteryReading {
    read_all().primary
}


#[cfg(test)]
mod tests {
    use super::*;

    fn taught_reading(percent: u8, verified: bool) -> DeviceReading {
        DeviceReading::taught(
            Brand::new("aula"),
            "Aula F75",
            "2.4 GHz",
            percent,
            verified,
        )
    }

    fn vendor_reading(percent: u8) -> DeviceReading {
        DeviceReading::ok(Brand::new("aula"), "Aula F75", "2.4 GHz", percent, false)
            .ranked(RANK_VENDOR)
    }

    fn windows_reading(percent: u8) -> DeviceReading {
        DeviceReading::ok(Brand::new("aula"), "Aula F75", "Bluetooth", percent, false)
    }

    /// The vendor frame is the hardware answering for itself; a taught byte is
    /// a location someone recognised by sight. Order must not depend on which
    /// reader happened to finish first.
    #[test]
    fn vendor_frame_beats_a_taught_byte_either_way_round() {
        let mut merged = Vec::new();
        push_unique(&mut merged, vendor_reading(80));
        push_unique(&mut merged, taught_reading(92, true));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].percent, Some(80));

        let mut merged = Vec::new();
        push_unique(&mut merged, taught_reading(92, true));
        push_unique(&mut merged, vendor_reading(80));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].percent, Some(80));
    }

    /// What the user confirmed still beats a value Windows cached when the
    /// device last paired — that is the case teaching exists for.
    #[test]
    fn a_taught_byte_beats_a_cached_windows_value() {
        let mut merged = Vec::new();
        push_unique(&mut merged, windows_reading(50));
        push_unique(&mut merged, taught_reading(92, true));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].percent, Some(92));
    }

    /// Once the evidence says the taught byte never moves, it stops winning.
    #[test]
    fn an_unverified_byte_loses_to_everything() {
        let mut merged = Vec::new();
        push_unique(&mut merged, taught_reading(92, false));
        push_unique(&mut merged, windows_reading(50));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].percent, Some(50));
    }

    /// The case the user hit: a receiver names itself one thing, the person who
    /// taught it typed another, so nothing merges by name and both cards stay.
    #[test]
    fn a_taught_byte_goes_once_its_hardware_answers_even_under_another_name() {
        let mut readings = vec![
            DeviceReading::ok(Brand::new("aula"), "Aula F75", "2.4 GHz", 78, false)
                .ranked(RANK_VENDOR)
                .measured_on(0x3554, 0xFA09),
            DeviceReading::taught(
                Brand::new("aula"),
                "2.4G Wireless Receiver",
                "2.4 GHz",
                92,
                true,
            )
            .measured_on(0x3554, 0xFA09),
        ];
        drop_superseded_taught(&mut readings);
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].percent, Some(78));
    }

    /// Teaching still carries hardware nothing else can read.
    #[test]
    fn a_taught_byte_stays_when_nothing_answers_for_it() {
        let mut readings = vec![
            DeviceReading::ok(Brand::new("razer"), "Headset", "2.4 GHz", 40, false)
                .ranked(RANK_VENDOR)
                .measured_on(0x1532, 0x0565),
            DeviceReading::taught(Brand::new("aula"), "Keyboard", "2.4 GHz", 92, true)
                .measured_on(0x3554, 0xFA09),
        ];
        drop_superseded_taught(&mut readings);
        assert_eq!(readings.len(), 2);
    }

    #[test]
    fn a_reading_replaces_a_presence_only_entry() {
        let mut merged = Vec::new();
        push_unique(
            &mut merged,
            DeviceReading::failed(Brand::new("aula"), "Aula F75", "2.4 GHz", "asleep", true),
        );
        push_unique(&mut merged, vendor_reading(80));
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].percent, Some(80));
    }
}
