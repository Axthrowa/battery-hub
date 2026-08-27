mod devices;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use devices::{BatteryReading, DeviceReading, DeviceSnapshot};
use devices::discover::DeviceCandidate;
use devices::learned::LearnedDevice;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, WebviewWindowBuilder, Window, WindowEvent, Wry,
};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_store::StoreExt;

/// Window creation itself emits resize events; only resizes while ready count as minimize.
static WINDOW_READY: AtomicBool = AtomicBool::new(false);

const TRAY_ID: &str = "battery-hub-tray";
const EVENT_BATTERY: &str = "battery://update";
const EVENT_DEVICES: &str = "devices://update";
const EVENT_OPEN_SETTINGS: &str = "ui://open-settings";
const DEFAULT_POLL_SECONDS: u64 = 60;
const MIN_POLL_SECONDS: u64 = 5;
const WATCH_POLL_SECONDS: u64 = 15;
/// A charge is the one time the number is expected to move, so it is watched
/// more closely than a device sitting on its own battery.
const CHARGING_POLL_SECONDS: u64 = 8;
const DISCONNECT_STRIKES: u32 = 2;
const ARG_REQUIRE_DONGLE: &str = "--require-dongle";
const ARG_SCAN_DEVICES: &str = "--scan-devices";
/// Per-device low-battery threshold (exclusive: notify when percent < this).
const LOW_BATTERY_THRESHOLD: u8 = 20;
/// Sounds the user supplied, kept beside the settings so they survive an
/// upgrade, and named plainly because a person may well go looking for them.
///
/// The two events are worth telling apart by ear: one says a cable can come
/// out, the other says a device is about to stop working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum SoundKind {
    Full,
    Low,
}

impl SoundKind {
    fn file_name(self) -> &'static str {
        match self {
            SoundKind::Full => "notification-full.wav",
            SoundKind::Low => "notification-low.wav",
        }
    }
}
/// Enough for any notification chime, and small enough that a wrong file
/// cannot fill the disk: a WAV this size is over a minute of CD audio.
const SOUND_FILE_MAX: usize = 5 * 1024 * 1024;

/// Windows' own notification chime. The name is the bare one the toast builder
/// parses — an `ms-winsoundevent:` URI does not match and is silently dropped.
/// Naming no sound at all is not the same thing: a toast built without one is
/// emitted with `<audio silent="true" />`, which is how the sound switch in
/// settings turns toasts quiet without hiding them.
const NOTIFICATION_SOUND: &str = "Default";
/// Close enough to full that a device dropping its charging flag means it has
/// finished rather than been unplugged early.
const FULL_ENOUGH: u8 = 95;
/// How long a charge has to sit still before it counts as done. Gauges on these
/// devices are read off the voltage, so a keyboard can stop at 99 and never
/// claim the last point — it charges to full and simply says 99 until the cable
/// comes out, at which point it settles back to what the cell really holds.
const FULL_STALL_MS: u64 = 15 * 60 * 1000;
/// How long a charging claim that has not yet been seen to gain a single point
/// may stand on nothing but its own word. A charge that is really happening
/// moves the gauge inside this: a mouse filling from empty gains a point every
/// minute or two. A receiver repeating a frame it heard an hour ago does not.
const CHARGE_GRACE_MS: u64 = 3 * 60 * 1000;
/// How long a charge that HAS been seen to climb may then sit still, short of
/// full, before it is treated as over rather than ongoing.
const CHARGE_STALL_MS: u64 = 10 * 60 * 1000;
/// How far the level may fall below the peak before the fall counts as real
/// rather than as gauge jitter. One point of wobble is ordinary on a gauge read
/// off the cell voltage; two in a row is the battery going down.
const CHARGE_DROP_TOLERANCE: u8 = 2;

/// What has been seen of one device's charge.
struct ChargeWatch {
    /// Highest level reached since the claim began, and when it got there.
    peak: u8,
    since_ms: u64,
    /// The level has been seen to rise while the claim stood — the one piece of
    /// evidence that separates a charge from a flag with nothing behind it.
    climbed: bool,
    /// The claim stopped being believed — see `judge_charge`.
    stale: bool,
    /// The device is claiming the cable right now. A watch is kept after the
    /// claim goes out, so the next one can be told apart from the first.
    active: bool,
}

/// What a device claiming the cable is actually doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Charge {
    /// On the cable and still taking a charge.
    Filling,
    /// On the cable with nothing left to take.
    Full,
    /// Says it is on the cable, but the level has not moved in long enough that
    /// the flag is no longer evidence of anything.
    Disbelieved,
}

/// Judge one device's charging claim against what has been seen of its level.
///
/// Split out from `apply_charge_state` because this is the whole of the
/// decision and none of the plumbing: no app handle, no toasts, no snapshot —
/// just the watch, the level and the clock, so it can be tested directly.
///
/// The claim by itself is worth very little. An Ajazz receiver serves the last
/// frame it heard, so it went on reporting "92%, charging" for a quarter of an
/// hour with the mouse on the desk — and over the hours before that the level
/// it reported walked *down*, 95 to 94 to 93 to 92, the whole time still
/// claiming the cable. So the level is the evidence and the flag is only the
/// question: a charge that is happening gains points, and one that is losing
/// them is not happening at all.
fn judge_charge(watch: &mut ChargeWatch, percent: u8, now: u64) -> Charge {
    if percent > watch.peak {
        // It gained: the flag belongs to a charge that is really under way.
        watch.peak = percent;
        watch.since_ms = now;
        watch.climbed = true;
        watch.stale = false;
    } else if watch.peak - percent >= CHARGE_DROP_TOLERANCE {
        // It lost charge while claiming the cable, which nothing on a charger
        // does. There is no waiting period for this one because there is
        // nothing left to wait for. The peak follows the level down so that a
        // charge starting later reads as a gain on the very next poll.
        watch.peak = percent;
        watch.since_ms = now;
        watch.stale = true;
    }

    let waited = now.saturating_sub(watch.since_ms);
    // A level that has stopped moving means one of two things, and which one
    // depends on whether it was ever seen to move at all: at the top of a gauge
    // that did climb, the charge is finished; anywhere else, the flag never had
    // anything behind it.
    let near_done = watch.climbed && percent >= FULL_ENOUGH;
    let patience = if watch.climbed {
        CHARGE_STALL_MS
    } else {
        CHARGE_GRACE_MS
    };
    if !near_done && waited >= patience {
        watch.stale = true;
    }

    if percent >= 100 {
        Charge::Full
    } else if watch.stale {
        Charge::Disbelieved
    } else if near_done && waited >= FULL_STALL_MS {
        Charge::Full
    } else {
        Charge::Filling
    }
}

/// Bring a kept watch back for a claim that has just come round again.
///
/// A device that really went on a charger in between says so in its level, and
/// `judge_charge` reads that as a gain on the next poll. One whose receiver is
/// repeating an old frame says nothing new, and must not buy another grace
/// period every time the flag flickers — that is the false bolt coming back a
/// few minutes at a time, which is exactly what it looks like from the panel.
fn reactivate(watch: &mut ChargeWatch, now: u64) {
    watch.active = true;
    if !watch.stale {
        watch.since_ms = now;
    }
}

/// Whether the flag going out means a charge just completed. A watch that was
/// never believed, or never saw the level rise, announces nothing: there is no
/// evidence a charge was happening, so there is none that one finished.
fn completed_on_unplug(watch: &ChargeWatch) -> bool {
    watch.climbed && !watch.stale && watch.peak >= FULL_ENOUGH
}

const SETTINGS_STORE_FILE: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";
const SESSION_STORE_FILE: &str = "session.json";
const SESSION_KEY: &str = "lastSession";

const DEVICE_ITEM_IDS: &[&str] = &[
    "dev-0", "dev-1", "dev-2", "dev-3", "dev-4", "dev-5", "dev-6", "dev-7",
];
const TRAY_DEVICE_SLOTS: usize = 8;

struct TrayItems {
    devices: Vec<MenuItem<Wry>>,
    show: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct Shared {
    poll_seconds: AtomicU64,
    refresh_requested: Mutex<bool>,
    wake: Condvar,
    locale: Mutex<String>,
    last: Mutex<Option<BatteryReading>>,
    last_snapshot: Mutex<Option<DeviceSnapshot>>,
    last_tooltip: Mutex<String>,
    last_tray_fingerprint: Mutex<String>,
    tray_items: Mutex<Option<TrayItems>>,
    connected: AtomicBool,
    /// Something is on a charger right now.
    charging: AtomicBool,
    misses: AtomicU32,
    dongle_seen: AtomicBool,
    dongle_misses: AtomicU32,
    /// Cleared as soon as the user opens the app by hand — see `launched_for_dongle`.
    exit_with_radio: AtomicBool,
    shutting_down: AtomicBool,
    /// Whether toasts are allowed to make a sound. Off leaves them visible but
    /// silent — Windows plays nothing at all when a toast names no sound.
    notify_sound: AtomicBool,
    /// Product keys that already fired a low-battery toast.
    low_battery_notified: Mutex<HashSet<String>>,
    /// Product keys that already fired a charge-complete toast.
    full_charge_notified: Mutex<HashSet<String>>,
    /// Highest level each product reached on the cable, and when it got there.
    charge_watch: Mutex<HashMap<String, ChargeWatch>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            poll_seconds: AtomicU64::new(DEFAULT_POLL_SECONDS),
            refresh_requested: Mutex::new(false),
            wake: Condvar::new(),
            locale: Mutex::new("tr".to_string()),
            last: Mutex::new(None),
            last_snapshot: Mutex::new(None),
            last_tooltip: Mutex::new(String::new()),
            last_tray_fingerprint: Mutex::new(String::new()),
            tray_items: Mutex::new(None),
            connected: AtomicBool::new(false),
            charging: AtomicBool::new(false),
            misses: AtomicU32::new(0),
            dongle_seen: AtomicBool::new(false),
            dongle_misses: AtomicU32::new(0),
            exit_with_radio: AtomicBool::new(launched_for_dongle()),
            shutting_down: AtomicBool::new(false),
            notify_sound: AtomicBool::new(true),
            low_battery_notified: Mutex::new(HashSet::new()),
            full_charge_notified: Mutex::new(HashSet::new()),
            charge_watch: Mutex::new(HashMap::new()),
        }
    }

    fn poll_interval(&self) -> Duration {
        let configured = self.poll_seconds.load(Ordering::Relaxed).max(MIN_POLL_SECONDS);
        let seconds = if self.charging.load(Ordering::Acquire) {
            configured.min(CHARGING_POLL_SECONDS)
        } else if self.connected.load(Ordering::Acquire) {
            configured.min(WATCH_POLL_SECONDS)
        } else {
            configured
        };
        Duration::from_secs(seconds)
    }

    fn nudge(&self, refresh: bool) {
        if refresh {
            *self.refresh_requested.lock().unwrap() = true;
        }
        self.wake.notify_all();
    }
}

fn tooltip_for(snapshot: Option<&DeviceSnapshot>) -> String {
    let Some(snap) = snapshot else {
        return "Battery Hub".to_string();
    };
    if snap.online.is_empty() {
        return "Battery Hub: offline".to_string();
    }
    snap.online
        .iter()
        .map(DeviceReading::tray_line)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn tray_fingerprint(snapshot: &DeviceSnapshot) -> String {
    snapshot
        .devices
        .iter()
        .map(|d| format!("{}:{}:{:?}", d.product, d.ok, d.percent))
        .collect::<Vec<_>>()
        .join(";")
}

fn apply_tray(app: &AppHandle, shared: &Shared) {
    let snapshot = shared.last_snapshot.lock().unwrap().clone();
    let tip = tooltip_for(snapshot.as_ref());
    let fingerprint = snapshot
        .as_ref()
        .map(tray_fingerprint)
        .unwrap_or_default();

    {
        let mut cached_tip = shared.last_tooltip.lock().unwrap();
        let mut cached_fp = shared.last_tray_fingerprint.lock().unwrap();
        if *cached_tip == tip && *cached_fp == fingerprint {
            return;
        }
        cached_tip.clone_from(&tip);
        cached_fp.clone_from(&fingerprint);
    }

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(&tip));
    }

    let Some(ref snap) = snapshot else {
        return;
    };
    if let Some(items) = shared.tray_items.lock().unwrap().as_ref() {
        let list: Vec<&DeviceReading> = if snap.online.is_empty() {
            snap.devices.iter().filter(|d| d.present).collect()
        } else {
            snap.online.iter().collect()
        };
        for (i, item) in items.devices.iter().enumerate() {
            if let Some(dev) = list.get(i) {
                let _ = item.set_text(dev.tray_line());
                let _ = item.set_enabled(dev.ok);
            } else if i == 0 && list.is_empty() {
                let _ = item.set_text("No devices");
                let _ = item.set_enabled(false);
            } else {
                let _ = item.set_text("—");
                let _ = item.set_enabled(false);
            }
        }
    }
}

fn main_window_alive(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some()
}

/// Close & Recreate: focus existing webview, otherwise rebuild from tauri.conf.json.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        WINDOW_READY.store(true, Ordering::Release);
        return;
    }

    let Some(config) = app.config().app.windows.first().cloned() else {
        return;
    };
    match WebviewWindowBuilder::from_config(app, &config).and_then(|builder| {
        builder.visible(true).focused(true).build()
    }) {
        Ok(window) => {
            let _ = window.set_focus();
            WINDOW_READY.store(true, Ordering::Release);
        }
        Err(err) => eprintln!("failed to recreate main window: {err}"),
    }
}

/// Destroys WebView2 via `close()` — never `hide()`. Host + poll stay in tray.
fn close_window_to_tray(window: &Window) {
    WINDOW_READY.store(false, Ordering::Release);
    let _ = window.close();
}

/// Tell Windows who `com.axthrowa.battery-hub` is.
///
/// Toasts are addressed by AppUserModelID, and an unpackaged app has to say
/// what its own ID stands for or the shell drops every notification without a
/// word — no error reaches the caller, the toast simply never appears. The
/// display name and icon here are what shows in Settings → Notifications, so
/// the user can find the app and turn it on.
#[cfg(windows)]
fn register_toast_identity(app: &AppHandle) {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegSetKeyValueW(
            key: isize,
            sub_key: *const u16,
            value: *const u16,
            kind: u32,
            data: *const core::ffi::c_void,
            bytes: u32,
        ) -> i32;
    }
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const REG_SZ: u32 = 1;

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let identifier = app.config().identifier.clone();
    let sub_key = wide(&format!("Software\\Classes\\AppUserModelId\\{identifier}"));
    let icon = std::env::current_exe()
        .map(|exe| format!("{},0", exe.display()))
        .unwrap_or_default();

    for (name, value) in [("DisplayName", "Battery Hub"), ("IconUri", icon.as_str())] {
        if value.is_empty() {
            continue;
        }
        let name = wide(name);
        let data = wide(value);
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                sub_key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status != 0 {
            eprintln!("toast identity: writing {status} failed");
        }
    }
}

fn sound_file_path(kind: SoundKind) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(std::path::PathBuf::from)?;
    let dir = base.join("Battery Hub");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(kind.file_name()))
}

/// Play a chosen sound file, if there is one, and say whether it was played.
///
/// Windows' toast XML can only name sounds the shell already knows, and a file
/// on disk is not one of them for an app that is not packaged: pointing a toast
/// at a path leaves it silent, without an error. So the toast is raised silent
/// on purpose and the file is played beside it, which from the other side of
/// the speakers is the same notification either way.
#[cfg(windows)]
fn play_sound_file(kind: SoundKind) -> bool {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u16, module: isize, flags: u32) -> i32;
    }
    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_FILENAME: u32 = 0x0002_0000;

    let Some(path) = sound_file_path(kind).filter(|p| p.is_file()) else {
        return false;
    };
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Asynchronous on purpose: a poll thread must not wait on a speaker.
    unsafe { PlaySoundW(wide.as_ptr(), 0, SND_FILENAME | SND_ASYNC | SND_NODEFAULT) != 0 }
}

#[cfg(not(windows))]
fn play_sound_file(_kind: SoundKind) -> bool {
    false
}

/// Give a toast its sound, whichever kind is in force.
///
/// A chosen file is played here and the toast is left silent; otherwise the
/// toast carries Windows' own chime. Either way the caller just builds and
/// shows it.
fn with_sound<R: tauri::Runtime>(
    shared: &Shared,
    builder: tauri_plugin_notification::NotificationBuilder<R>,
    kind: SoundKind,
) -> tauri_plugin_notification::NotificationBuilder<R> {
    if !shared.notify_sound.load(Ordering::Relaxed) {
        return builder;
    }
    if play_sound_file(kind) {
        return builder;
    }
    builder.sound(NOTIFICATION_SOUND)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn notify_connected(app: &AppHandle, shared: &Shared, snapshot: &DeviceSnapshot) {
    let tr = shared.locale.lock().unwrap().starts_with("tr");
    let summary = snapshot
        .online
        .iter()
        .map(DeviceReading::tray_line)
        .collect::<Vec<_>>()
        .join(", ");
    let (title, body) = if tr {
        ("Cihaz bağlandı".to_string(), summary)
    } else {
        ("Device connected".to_string(), summary)
    };
    if let Err(err) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification failed: {err}");
    }
}

/// Fires once per product when SOC drops below the threshold, and again only
/// after it recovers. Format: "{product} low battery".
fn notify_low_battery(app: &AppHandle, shared: &Shared, snapshot: &DeviceSnapshot) {
    let mut notified = shared.low_battery_notified.lock().unwrap();
    for device in &snapshot.devices {
        let key = device.product.to_ascii_lowercase();
        let Some(percent) = device.percent.filter(|_| device.ok) else {
            notified.remove(&key);
            continue;
        };
        if percent >= LOW_BATTERY_THRESHOLD {
            notified.remove(&key);
            continue;
        }
        if !notified.insert(key.clone()) {
            continue;
        }
        let product = if device.product.trim().is_empty() {
            device.brand_label.clone()
        } else {
            device.product.clone()
        };
        let body = format!("{product} low battery");
        let toast = with_sound(
            shared,
            app.notification().builder().title("Battery Hub").body(&body),
            SoundKind::Low,
        );
        match toast.show() {
            Ok(()) => {
                devices::diagnostics::emit_line(&format!("[notify] sent: {body} ({percent}%)"))
            }
            Err(err) => {
                devices::diagnostics::emit_line(&format!("[notify] FAILED: {body} — {err}"));
                notified.remove(&key);
            }
        }
    }
}

/// Decide what "charging" means for each device this poll, and say so once it
/// is over.
///
/// Readers only report the flag the hardware sets, and hardware is unhelpful
/// here in three different ways. Some devices sit at 100% still calling
/// themselves charging. Others read their level off the cell voltage, top out
/// at 99 and never move again — the charge is done, the number just cannot say
/// so until the cable comes out and the reading settles back. And a 2.4 GHz
/// receiver keeps serving the last frame it heard from a device that has since
/// gone off the air, so a charging bit set during some earlier moment on the
/// dock is repeated for hours after the device is back on the desk.
///
/// All three are recognised the same way: by watching whether the level moves.
/// A charge that is really happening gains points. One that claims the cable
/// and has not gained a point in ten minutes, while nowhere near full, is not
/// describing this moment and is not shown as charging.
fn apply_charge_state(app: &AppHandle, shared: &Shared, snapshot: &mut DeviceSnapshot) {
    let now = now_ms();
    let mut watch = shared.charge_watch.lock().unwrap();
    let mut finished: Vec<String> = Vec::new();

    for device in &mut snapshot.devices {
        let key = device.product.to_ascii_lowercase();
        let Some(percent) = device.percent.filter(|_| device.ok) else {
            continue;
        };

        if device.charging {
            let entry = watch.entry(key.clone()).or_insert(ChargeWatch {
                peak: percent,
                since_ms: now,
                climbed: false,
                stale: false,
                active: true,
            });
            if !entry.active {
                reactivate(entry, now);
            }
            match judge_charge(entry, percent, now) {
                Charge::Full => {
                    device.full = true;
                    finished.push(key);
                }
                Charge::Filling => device.full = false,
                Charge::Disbelieved => {
                    device.charging = false;
                    device.full = false;
                }
            }
        } else if let Some(entry) = watch.get_mut(&key) {
            device.full = false;
            if entry.active {
                entry.active = false;
                // Off the cable: it finished if it had got near full while on
                // it. The level itself is no use at this moment — a voltage
                // gauge drops the instant the charger stops holding it up.
                if completed_on_unplug(entry) {
                    finished.push(key.clone());
                }
            }
            // A watch that was believed is finished business. A disbelieved one
            // is remembered, so the same echo cannot present itself as a fresh
            // claim; only the level rising clears it.
            if !entry.stale {
                watch.remove(&key);
            }
        } else {
            device.full = false;
        }
    }
    drop(watch);

    snapshot.online = snapshot.devices.iter().filter(|d| d.ok).cloned().collect();
    // Devices that have finished stay on the fast cadence rather than dropping
    // to the idle one: a finished charge is precisely the state someone is
    // waiting to see end, and the cable coming out is what ends it.
    shared.charging.store(
        snapshot.devices.iter().any(|d| d.charging),
        Ordering::Release,
    );
    notify_full_charge(app, shared, snapshot, &finished);
}

/// Fires once per product when its charge completes, and again only after it
/// has been used enough to need another one. Audible on purpose: the point is
/// to say the cable can come out without anyone watching the panel for it.
fn notify_full_charge(
    app: &AppHandle,
    shared: &Shared,
    snapshot: &DeviceSnapshot,
    finished: &[String],
) {
    let tr = shared.locale.lock().unwrap().starts_with("tr");
    let mut notified = shared.full_charge_notified.lock().unwrap();

    // Anything charging again, or run down since, may announce itself afresh.
    for device in &snapshot.devices {
        let key = device.product.to_ascii_lowercase();
        let draining = device.percent.is_some_and(|p| p < FULL_ENOUGH);
        if draining && !device.charging {
            notified.remove(&key);
        }
    }

    for key in finished {
        if !notified.insert(key.clone()) {
            continue;
        }
        let product = snapshot
            .devices
            .iter()
            .find(|d| d.product.to_ascii_lowercase() == *key)
            .map(|d| {
                if d.product.trim().is_empty() {
                    d.brand_label.clone()
                } else {
                    d.product.clone()
                }
            })
            .unwrap_or_else(|| key.clone());
        let (title, body) = if tr {
            ("Şarj tamamlandı".to_string(), format!("{product} tam dolu"))
        } else {
            (
                "Charging complete".to_string(),
                format!("{product} is fully charged"),
            )
        };
        let toast = with_sound(
            shared,
            app.notification().builder().title(title).body(&body),
            SoundKind::Full,
        );
        match toast.show() {
            Ok(()) => devices::diagnostics::emit_line(&format!("[notify] sent: {body}")),
            Err(err) => {
                devices::diagnostics::emit_line(&format!("[notify] FAILED: {body} — {err}"));
                notified.remove(key);
            }
        }
    }
}

fn store_write(app: &AppHandle, key: &str, value: serde_json::Value) {
    match app.store(SESSION_STORE_FILE) {
        Ok(store) => {
            store.set(key, value);
            if let Err(err) = store.save() {
                eprintln!("session store save failed: {err}");
            }
        }
        Err(err) => eprintln!("session store unavailable: {err}"),
    }
}

fn load_settings_from_store(app: &AppHandle, shared: &Shared) {
    let Ok(store) = app.store(SETTINGS_STORE_FILE) else {
        return;
    };
    let Some(value) = store.get(SETTINGS_KEY) else {
        return;
    };
    if let Some(locale) = value.get("locale").and_then(|v| v.as_str()) {
        *shared.locale.lock().unwrap() = locale.to_string();
    }
    if let Some(seconds) = value.get("pollSeconds").and_then(|v| v.as_u64()) {
        shared
            .poll_seconds
            .store(seconds.clamp(MIN_POLL_SECONDS, 3600), Ordering::Relaxed);
    }
    if let Some(on) = value.get("notificationSound").and_then(|v| v.as_bool()) {
        shared.notify_sound.store(on, Ordering::Relaxed);
    }
}

/// Windows keeps the login entry as a full path to the executable, and it is
/// written once — when the switch is flipped. Anything that moves the binary
/// (an install over a portable copy, a version that lands in a new folder, a
/// build that was toggled on from `target\debug`) leaves the entry pointing at
/// a path that no longer exists, and Windows then skips it without a word: the
/// switch still reads "on" while nothing starts. Re-asserting the stored
/// preference on every launch keeps the entry pointing at the running binary.
#[cfg(desktop)]
fn reconcile_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;

    let Some(wanted) = app
        .store(SETTINGS_STORE_FILE)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .and_then(|value| value.get("autostart").and_then(|flag| flag.as_bool()))
    else {
        return;
    };

    let manager = app.autolaunch();
    let outcome = if wanted {
        manager.enable()
    } else if manager.is_enabled().unwrap_or(false) {
        manager.disable()
    } else {
        Ok(())
    };
    if let Err(err) = outcome {
        eprintln!("autostart reconcile failed: {err}");
    }
}

fn flush_state(app: &AppHandle, reason: &str) -> bool {
    let Some(shared) = app.try_state::<Arc<Shared>>().map(|s| s.inner().clone()) else {
        return false;
    };
    if shared.shutting_down.swap(true, Ordering::AcqRel) {
        return false;
    }

    let payload = serde_json::json!({
        "reason": reason,
        "exitedAtMs": now_ms(),
        "lastReading": shared.last.lock().unwrap().clone(),
        "lastSnapshot": shared.last_snapshot.lock().unwrap().clone(),
    });

    store_write(app, SESSION_KEY, payload);
    true
}

fn graceful_shutdown(app: &AppHandle, reason: &str) {
    flush_state(app, reason);
    app.exit(0);
}

/// True only for the launch the device-arrival task makes (see
/// `scripts/device-trigger`). That instance exists to watch one receiver and is
/// meant to exit with it, because a scheduled task starts it again on the next
/// plug-in. A normal or autostart launch has no such task behind it: quitting
/// there means the tray icon disappears minutes after login — as soon as the
/// mouse sleeps or the headset is switched off — and never comes back.
fn launched_for_dongle() -> bool {
    std::env::args().any(|arg| arg == ARG_REQUIRE_DONGLE)
}

fn poll_loop(app: AppHandle, shared: Arc<Shared>) {
    loop {
        // Teşhis: tüm HID VID/PID + Bluetooth adları (konsol + diagnostics.log).
        devices::diagnostics::run_poll_diagnostics();

        // Four concurrent brand reader threads (see devices::read_all).
        let mut snapshot = devices::read_all();
        apply_charge_state(&app, &shared, &mut snapshot);
        for d in &snapshot.devices {
            let line = match (d.ok, d.percent) {
                (true, Some(p)) => format!(
                    "[poll] {} OK {}%{}{} ({}) {}",
                    d.brand.label(),
                    p,
                    if d.full {
                        " FULL"
                    } else if d.charging {
                        " CHARGING"
                    } else {
                        ""
                    },
                    if d.unverified { " UNVERIFIED" } else { "" },
                    d.transport,
                    d.product
                ),
                _ => format!(
                    "[poll] {} FAIL present={} err={}",
                    d.brand.label(),
                    d.present,
                    d.error.as_deref().unwrap_or("-")
                ),
            };
            devices::diagnostics::emit_line(&line);
        }
        *shared.last.lock().unwrap() = Some(snapshot.primary.clone());
        *shared.last_snapshot.lock().unwrap() = Some(snapshot.clone());
        apply_tray(&app, &shared);
        notify_low_battery(&app, &shared, &snapshot);

        if main_window_alive(&app) {
            let _ = app.emit(EVENT_BATTERY, &snapshot.primary);
            let _ = app.emit(EVENT_DEVICES, &snapshot);
        }

        if !snapshot.online.is_empty() {
            shared.misses.store(0, Ordering::Relaxed);
            if !shared.connected.swap(true, Ordering::AcqRel) {
                notify_connected(&app, &shared, &snapshot);
            }
        } else if shared.connected.load(Ordering::Acquire)
            && shared.misses.fetch_add(1, Ordering::AcqRel) + 1 >= DISCONNECT_STRIKES
        {
            shared.connected.store(false, Ordering::Release);
            shared.misses.store(0, Ordering::Relaxed);
        }

        let radio = snapshot.devices.iter().any(|d| d.present);
        if radio {
            shared.dongle_misses.store(0, Ordering::Relaxed);
            shared.dongle_seen.store(true, Ordering::Release);
        } else if shared.exit_with_radio.load(Ordering::Acquire)
            && shared.dongle_seen.load(Ordering::Acquire)
            && shared.dongle_misses.fetch_add(1, Ordering::AcqRel) + 1 >= DISCONNECT_STRIKES
        {
            graceful_shutdown(&app, "radio-removed");
            return;
        }

        wait_for_next_poll(&shared);
    }
}

fn wait_for_next_poll(shared: &Shared) {
    let started = Instant::now();
    let mut requested = shared.refresh_requested.lock().unwrap();
    while !*requested {
        let Some(remaining) = shared.poll_interval().checked_sub(started.elapsed()) else {
            break;
        };
        let (guard, _) = shared
            .wake
            .wait_timeout(requested, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        requested = guard;
    }
    *requested = false;
}

/// Deep scan for hardware whose battery byte the user can confirm by sight.
#[tauri::command(async)]
fn scan_devices() -> Vec<DeviceCandidate> {
    devices::discover::scan()
}

#[tauri::command]
fn learned_devices() -> Vec<LearnedDevice> {
    devices::learned::all()
}

#[tauri::command]
fn add_learned_device(
    state: State<'_, Arc<Shared>>,
    device: LearnedDevice,
) -> Result<Vec<LearnedDevice>, String> {
    let list = devices::learned::add(device)?;
    state.nudge(true);
    Ok(list)
}

#[tauri::command]
fn remove_learned_device(
    state: State<'_, Arc<Shared>>,
    id: String,
) -> Result<Vec<LearnedDevice>, String> {
    let list = devices::learned::remove(&id)?;
    state.nudge(true);
    Ok(list)
}

#[tauri::command]
fn last_reading(state: State<'_, Arc<Shared>>) -> Option<BatteryReading> {
    state.last.lock().unwrap().clone()
}

#[tauri::command]
fn last_devices(state: State<'_, Arc<Shared>>) -> Option<DeviceSnapshot> {
    state.last_snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn refresh_now(state: State<'_, Arc<Shared>>) {
    state.nudge(true);
}

#[tauri::command]
fn set_poll_seconds(state: State<'_, Arc<Shared>>, seconds: u64) {
    state
        .poll_seconds
        .store(seconds.clamp(MIN_POLL_SECONDS, 3600), Ordering::Relaxed);
    state.nudge(false);
}

#[tauri::command]
fn set_notification_sound(state: State<'_, Arc<Shared>>, enabled: bool) {
    state.notify_sound.store(enabled, Ordering::Relaxed);
}

/// Store a sound the user picked, or clear it when given nothing.
#[tauri::command]
fn set_notification_sound_file(kind: SoundKind, data: Option<Vec<u8>>) -> Result<bool, String> {
    let path = sound_file_path(kind).ok_or("No writable app data directory.")?;
    match data {
        Some(bytes) => {
            if bytes.is_empty() {
                return Err("The file is empty.".into());
            }
            if bytes.len() > SOUND_FILE_MAX {
                return Err("The file is larger than 5 MB.".into());
            }
            // RIFF/WAVE is what PlaySound plays; anything else would be stored
            // and then quietly do nothing, which is worse than saying so.
            if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
                return Err("Only WAV files can be played.".into());
            }
            std::fs::write(&path, bytes).map_err(|err| err.to_string())?;
            Ok(true)
        }
        None => {
            let _ = std::fs::remove_file(&path);
            Ok(false)
        }
    }
}

/// Play what a notification would play, so the choice can be heard before one
/// arrives — which is otherwise a matter of waiting for a battery to run down.
#[tauri::command]
fn test_notification_sound(app: AppHandle, state: State<'_, Arc<Shared>>, kind: SoundKind) {
    let tr = state.locale.lock().unwrap().starts_with("tr");
    let (title, body) = match (tr, kind) {
        (true, SoundKind::Full) => ("Battery Hub", "Şarj tamamlandı — ses denemesi"),
        (true, SoundKind::Low) => ("Battery Hub", "Düşük pil — ses denemesi"),
        (false, SoundKind::Full) => ("Battery Hub", "Charging complete — sound test"),
        (false, SoundKind::Low) => ("Battery Hub", "Low battery — sound test"),
    };
    let toast = with_sound(
        &state,
        app.notification().builder().title(title).body(body),
        kind,
    );
    if let Err(err) = toast.show() {
        devices::diagnostics::emit_line(&format!("[notify] test FAILED — {err}"));
    }
}

#[tauri::command]
fn apply_localization(
    app: AppHandle,
    state: State<'_, Arc<Shared>>,
    locale: String,
    show: String,
    settings: String,
    quit: String,
) {
    *state.locale.lock().unwrap() = locale;
    if let Some(items) = state.tray_items.lock().unwrap().as_ref() {
        let _ = items.show.set_text(show);
        let _ = items.settings.set_text(settings);
        let _ = items.quit.set_text(quit);
    }
    apply_tray(&app, &state);
}

#[tauri::command]
fn close_to_tray(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        close_window_to_tray(&window.as_ref().window_ref());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Support path: run the add-device scan, write the result to
    // diagnostics.log and exit without starting the UI.
    if std::env::args().any(|arg| arg == ARG_SCAN_DEVICES) {
        let started = Instant::now();
        let found = devices::discover::scan();
        devices::diagnostics::emit_line(&format!(
            "[scan] {} candidate(s) in {:.1}s",
            found.len(),
            started.elapsed().as_secs_f32()
        ));
        for candidate in &found {
            devices::diagnostics::emit_line(&format!(
                "[scan] {}",
                serde_json::to_string(candidate).unwrap_or_default()
            ));
        }
        return;
    }

    let shared = Arc::new(Shared::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|arg| arg == ARG_REQUIRE_DONGLE) {
                return;
            }
            // Opened by hand: this is no longer a watcher the device-arrival
            // task may throw away when the receiver goes.
            if let Some(shared) = app.try_state::<Arc<Shared>>() {
                shared.exit_with_radio.store(false, Ordering::Release);
            }
            show_main_window(app);
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            scan_devices,
            learned_devices,
            add_learned_device,
            remove_learned_device,
            last_reading,
            last_devices,
            refresh_now,
            set_poll_seconds,
            set_notification_sound,
            set_notification_sound_file,
            test_notification_sound,
            apply_localization,
            close_to_tray
        ])
        .on_window_event(|window, event| match event {
            // Let close proceed so WebView2 is destroyed. Process stays via prevent_exit.
            WindowEvent::CloseRequested { .. } => {
                WINDOW_READY.store(false, Ordering::Release);
            }
            WindowEvent::Destroyed => {
                WINDOW_READY.store(false, Ordering::Release);
            }
            // Minimize → destroy webview (never hide).
            WindowEvent::Resized(_)
                if WINDOW_READY.load(Ordering::Acquire)
                    && window.is_minimized().unwrap_or(false)
                    && window.is_visible().unwrap_or(false) =>
            {
                let _ = window.unminimize();
                close_window_to_tray(window);
            }
            _ => {}
        })
        .setup(move |app| {
            if std::env::args().any(|arg| arg == ARG_REQUIRE_DONGLE) && !devices::any_radio_present()
            {
                std::process::exit(0);
            }

            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::MacosLauncher;
                app.handle().plugin(tauri_plugin_autostart::init(
                    MacosLauncher::LaunchAgent,
                    Some(vec!["--minimized"]),
                ))?;
                reconcile_autostart(app.handle());
                #[cfg(windows)]
                register_toast_identity(app.handle());
            }

            // Bildirim izni yoksa iste; aksi halde `.show()` sessizce başarısız olur
            // ve düşük pil uyarısı hiç görünmez.
            {
                let notifier = app.notification();
                match notifier.permission_state() {
                    Ok(PermissionState::Granted) => {}
                    Ok(_) => {
                        if let Err(err) = notifier.request_permission() {
                            eprintln!("notification permission request failed: {err}");
                        }
                    }
                    Err(err) => eprintln!("notification permission state unavailable: {err}"),
                }
            }

            let mut device_items = Vec::with_capacity(TRAY_DEVICE_SLOTS);
            for id in DEVICE_ITEM_IDS.iter().take(TRAY_DEVICE_SLOTS) {
                device_items.push(MenuItem::with_id(app, *id, "—", false, None::<&str>)?);
            }
            let show = MenuItem::with_id(app, "show", "Göster", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Ayarlar", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let head_separator = PredefinedMenuItem::separator(app)?;

            let menu = Menu::with_items(
                app,
                &[
                    &device_items[0],
                    &device_items[1],
                    &device_items[2],
                    &device_items[3],
                    &device_items[4],
                    &device_items[5],
                    &device_items[6],
                    &device_items[7],
                    &head_separator,
                    &show,
                    &settings,
                    &separator,
                    &quit,
                ],
            )?;

            let state = app.state::<Arc<Shared>>().inner().clone();
            load_settings_from_store(app.handle(), &state);
            if !state.locale.lock().unwrap().starts_with("tr") {
                let _ = show.set_text("Show");
                let _ = settings.set_text("Settings");
                let _ = quit.set_text("Quit");
            }
            *state.tray_items.lock().unwrap() = Some(TrayItems {
                devices: device_items,
                show: show.clone(),
                settings: settings.clone(),
                quit: quit.clone(),
            });

            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Battery Hub")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "settings" => {
                        show_main_window(app);
                        let _ = app.emit(EVENT_OPEN_SETTINGS, ());
                    }
                    "quit" => graceful_shutdown(app, "user-quit"),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                    | TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => {
                        show_main_window(tray.app_handle());
                    }
                    _ => {}
                });

            // The battery artwork is tall, and the tray renders it at 16 px:
            // use the square-cropped variant so it does not shrink to a sliver.
            const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
            match tauri::image::Image::from_bytes(TRAY_ICON) {
                Ok(icon) => tray = tray.icon(icon),
                Err(err) => {
                    eprintln!("tray icon unavailable, falling back: {err}");
                    if let Some(icon) = app.default_window_icon().cloned() {
                        tray = tray.icon(icon);
                    }
                }
            }
            tray.build(app)?;

            // Silent start: destroy the hidden config window so WebView2 is not resident.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.close();
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || poll_loop(handle, state));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            RunEvent::ExitRequested { code, api, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            RunEvent::Exit => {
                flush_state(app, "process-exit");
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60 * 1000;

    fn watch_at(percent: u8, now: u64) -> ChargeWatch {
        ChargeWatch {
            peak: percent,
            since_ms: now,
            climbed: false,
            stale: false,
            active: true,
        }
    }

    #[test]
    fn a_charge_that_climbs_keeps_its_flag() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(60, start);
        for (minute, percent) in [(2u64, 61u8), (4, 62), (7, 63), (30, 70)] {
            let now = start + minute * MINUTE;
            assert_eq!(judge_charge(&mut watch, percent, now), Charge::Filling);
        }
        assert!(watch.climbed);
    }

    /// The Ajazz case, from its own log: the receiver repeated "92%, charging"
    /// for a quarter of an hour with the mouse on the desk.
    #[test]
    fn a_claim_that_never_gains_a_point_stops_being_believed() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(92, start);
        assert_eq!(judge_charge(&mut watch, 92, start + MINUTE), Charge::Filling);
        assert_eq!(
            judge_charge(&mut watch, 92, start + CHARGE_GRACE_MS),
            Charge::Disbelieved
        );
        // And stays disbelieved rather than flapping back a window later.
        assert_eq!(
            judge_charge(&mut watch, 92, start + 5 * CHARGE_GRACE_MS),
            Charge::Disbelieved
        );
    }

    /// Also from the log: 95 → 94 → 93 → 92, claiming the cable throughout.
    /// Nothing on a charger loses charge, so this needs no waiting period.
    #[test]
    fn a_level_falling_under_the_claim_ends_it_at_once() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(95, start);
        assert_eq!(judge_charge(&mut watch, 95, start + MINUTE), Charge::Filling);
        assert_eq!(
            judge_charge(&mut watch, 93, start + 2 * MINUTE),
            Charge::Disbelieved
        );
    }

    /// One point of wobble is ordinary on a gauge read off the cell voltage.
    #[test]
    fn a_single_point_of_wobble_is_not_a_fall() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(60, start);
        assert_eq!(judge_charge(&mut watch, 61, start + MINUTE), Charge::Filling);
        assert_eq!(
            judge_charge(&mut watch, 60, start + 2 * MINUTE),
            Charge::Filling
        );
    }

    #[test]
    fn a_disbelieved_claim_is_taken_back_once_the_level_gains() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(80, start);
        assert_eq!(
            judge_charge(&mut watch, 80, start + CHARGE_GRACE_MS),
            Charge::Disbelieved
        );
        let moved = start + CHARGE_GRACE_MS + MINUTE;
        assert_eq!(judge_charge(&mut watch, 81, moved), Charge::Filling);
    }

    /// A device that drained while the flag was stuck on must not need to climb
    /// all the way back to the old peak before a real charge is believed.
    #[test]
    fn a_charge_after_a_drain_is_believed_from_the_new_level() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(95, start);
        assert_eq!(
            judge_charge(&mut watch, 88, start + MINUTE),
            Charge::Disbelieved
        );
        assert_eq!(watch.peak, 88, "the peak follows the level down");
        assert_eq!(
            judge_charge(&mut watch, 89, start + 2 * MINUTE),
            Charge::Filling
        );
    }

    /// A gauge that tops out below 100 is why `FULL_ENOUGH` exists: once it has
    /// been seen to climb, that stall is a finished charge.
    #[test]
    fn a_stall_near_full_after_a_real_climb_is_a_finished_charge() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(97, start);
        assert_eq!(judge_charge(&mut watch, 99, start + MINUTE), Charge::Filling);
        let settled = start + MINUTE + FULL_STALL_MS;
        assert_eq!(judge_charge(&mut watch, 99, settled), Charge::Full);
    }

    /// The phantom the log shows: 95% sat there claiming the cable, never
    /// climbed, and the old rule announced a charge that never happened.
    #[test]
    fn a_stall_near_full_that_never_climbed_announces_nothing() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(95, start);
        assert_eq!(
            judge_charge(&mut watch, 95, start + FULL_STALL_MS),
            Charge::Disbelieved
        );
        assert!(!completed_on_unplug(&watch));
    }

    /// The Ajazz flag does not sit still: it goes out for a poll or two and
    /// comes back. Handing each return a fresh grace period puts the bolt back
    /// on the card for three minutes at a time, forever.
    #[test]
    fn a_flickering_claim_does_not_buy_another_grace_period() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(92, start);
        assert_eq!(
            judge_charge(&mut watch, 92, start + CHARGE_GRACE_MS),
            Charge::Disbelieved
        );

        // The flag drops, then comes back an hour later on the same level.
        watch.active = false;
        let again = start + 60 * MINUTE;
        reactivate(&mut watch, again);
        assert_eq!(
            judge_charge(&mut watch, 92, again),
            Charge::Disbelieved,
            "the same echo must not read as a fresh claim"
        );
    }

    /// But a charge that really starts later still gets believed: the level is
    /// what clears it, and the level is what a real charge moves.
    #[test]
    fn a_real_charge_after_a_flicker_is_still_believed() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(92, start);
        judge_charge(&mut watch, 92, start + CHARGE_GRACE_MS);
        watch.active = false;
        let again = start + 60 * MINUTE;
        reactivate(&mut watch, again);
        assert_eq!(judge_charge(&mut watch, 93, again), Charge::Filling);
    }

    #[test]
    fn a_hundred_percent_is_full_however_it_got_there() {
        let start = 10 * MINUTE;
        let mut watch = watch_at(96, start);
        assert_eq!(judge_charge(&mut watch, 100, start + MINUTE), Charge::Full);
    }

    #[test]
    fn unplugging_after_a_real_charge_announces_it() {
        let mut watch = watch_at(97, 0);
        watch.climbed = true;
        assert!(completed_on_unplug(&watch));
        watch.peak = 60;
        assert!(!completed_on_unplug(&watch));
    }

    #[test]
    fn unplugging_after_a_disbelieved_claim_announces_nothing() {
        let mut watch = watch_at(97, 0);
        watch.climbed = true;
        watch.stale = true;
        assert!(!completed_on_unplug(&watch));
    }
}
