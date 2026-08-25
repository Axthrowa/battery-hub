mod devices;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use devices::{BatteryReading, DeviceReading, DeviceSnapshot};
use devices::ble_gatt::BleBatteryInfo;
use devices::discover::DeviceCandidate;
use devices::learned::LearnedDevice;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, WebviewWindowBuilder, Window, WindowEvent, Wry,
};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_store::StoreExt;

/// Window creation itself emits resize events; only resizes while ready count as minimize.
static WINDOW_READY: AtomicBool = AtomicBool::new(false);

const TRAY_ID: &str = "battery-hub-tray";
const EVENT_BATTERY: &str = "battery://update";
const EVENT_DEVICES: &str = "devices://update";
const EVENT_OPEN_SETTINGS: &str = "ui://open-settings";
const EVENT_BEFORE_EXIT: &str = "app://before-exit";
const DEFAULT_POLL_SECONDS: u64 = 60;
const MIN_POLL_SECONDS: u64 = 5;
const WATCH_POLL_SECONDS: u64 = 15;
const DISCONNECT_STRIKES: u32 = 2;
const ARG_REQUIRE_DONGLE: &str = "--require-dongle";
const ARG_SCAN_DEVICES: &str = "--scan-devices";
const SHUTDOWN_GRACE_MS: u64 = 250;
/// Per-device low-battery threshold (exclusive: notify when percent < this).
const LOW_BATTERY_THRESHOLD: u8 = 20;

const SETTINGS_STORE_FILE: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";
const SESSION_STORE_FILE: &str = "session.json";
const SESSION_KEY: &str = "lastSession";
const SESSION_FRONTEND_KEY: &str = "frontendState";

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
    misses: AtomicU32,
    dongle_seen: AtomicBool,
    dongle_misses: AtomicU32,
    shutting_down: AtomicBool,
    /// Product keys that already fired a low-battery toast.
    low_battery_notified: Mutex<HashSet<String>>,
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
            misses: AtomicU32::new(0),
            dongle_seen: AtomicBool::new(false),
            dongle_misses: AtomicU32::new(0),
            shutting_down: AtomicBool::new(false),
            low_battery_notified: Mutex::new(HashSet::new()),
        }
    }

    fn poll_interval(&self) -> Duration {
        let configured = self.poll_seconds.load(Ordering::Relaxed).max(MIN_POLL_SECONDS);
        let seconds = if self.connected.load(Ordering::Acquire) {
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
        devices::diagnostics::emit_line(&format!("[notify] {body} ({percent}%)"));
        if let Err(err) = app
            .notification()
            .builder()
            .title("Battery Hub")
            .body(&body)
            .show()
        {
            eprintln!("low-battery notification failed: {err}");
            notified.remove(&key);
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

    if app.get_webview_window("main").is_some() {
        let _ = app.emit(EVENT_BEFORE_EXIT, &payload);
        std::thread::sleep(Duration::from_millis(SHUTDOWN_GRACE_MS));
    }

    store_write(app, SESSION_KEY, payload);
    true
}

fn graceful_shutdown(app: &AppHandle, reason: &str) {
    flush_state(app, reason);
    app.exit(0);
}

fn poll_loop(app: AppHandle, shared: Arc<Shared>) {
    loop {
        // Teşhis: tüm HID VID/PID + Bluetooth adları (konsol + diagnostics.log).
        devices::diagnostics::run_poll_diagnostics();

        // Four concurrent brand reader threads (see devices::read_all).
        let snapshot = devices::read_all();
        for d in &snapshot.devices {
            let line = match (d.ok, d.percent) {
                (true, Some(p)) => format!(
                    "[poll] {} OK {}% ({}) {}",
                    d.brand.label(),
                    p,
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
        } else if shared.dongle_seen.load(Ordering::Acquire)
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

#[tauri::command(async)]
fn get_battery() -> BatteryReading {
    devices::read_battery()
}

#[tauri::command(async)]
fn get_devices() -> DeviceSnapshot {
    devices::read_all()
}

/// WinRT BLE scan: GATT Battery Service (UUID 0x180F) → Battery Level (0x2A19).
#[tauri::command(async)]
fn read_bluetooth_battery() -> Vec<BleBatteryInfo> {
    devices::ble_gatt::scan_ble_batteries()
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

#[tauri::command]
fn quit_app(app: AppHandle) {
    graceful_shutdown(&app, "user-quit");
}

#[tauri::command]
fn save_session(app: AppHandle, data: serde_json::Value) {
    store_write(&app, SESSION_FRONTEND_KEY, data);
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
            if !argv.iter().any(|arg| arg == ARG_REQUIRE_DONGLE) {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            get_battery,
            get_devices,
            read_bluetooth_battery,
            scan_devices,
            learned_devices,
            add_learned_device,
            remove_learned_device,
            last_reading,
            last_devices,
            refresh_now,
            set_poll_seconds,
            apply_localization,
            close_to_tray,
            quit_app,
            save_session
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
            WindowEvent::Resized(_) => {
                if WINDOW_READY.load(Ordering::Acquire)
                    && window.is_minimized().unwrap_or(false)
                    && window.is_visible().unwrap_or(false)
                {
                    let _ = window.unminimize();
                    close_window_to_tray(window);
                }
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
