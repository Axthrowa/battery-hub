//! Multi-brand battery hub — specialized readers + generic Windows/BLE/HID discovery.

mod ajazz;
mod brand;
pub mod ble_gatt;
pub mod diagnostics;
pub mod hid;
mod hid_battery;
mod hid_descriptor;
mod logitech;
mod razer;
pub mod soundcore;
mod windows_battery;

pub use brand::Brand;

use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
            transport: transport.into(),
            product: product.into(),
            error: None,
            present: true,
            updated_at_ms: now_ms(),
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
            transport: transport.into(),
            product: product.into(),
            error: Some(error.into()),
            present,
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
            (true, true) => {
                // Prefer specialized transports over generic Bluetooth when both OK.
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

    let _ = h_r.join();
    let _ = h_l.join();
    let _ = h_a.join();
    let _ = h_s.join();
    let _ = h_w.join();
    let _ = h_g.join();
    let _ = h_h.join();

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
    razer::dongle_present() || logitech::receiver_present() || ajazz::receiver_present()
}

/// Primary device across every brand — not just the Razer link.
pub fn read_battery() -> BatteryReading {
    read_all().primary
}
