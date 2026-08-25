mod battery;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use battery::BatteryReading;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, WebviewWindowBuilder, Window, WindowEvent, Wry,
};

/// Pushes the process working set out to the pagefile so the WebView2 render
/// process stops holding RAM while the window is hidden in the tray.
#[cfg(windows)]
mod winmem {
    use std::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_SET_QUOTA: u32 = 0x0100;
    const MAX_PATH: usize = 260;

    #[repr(C)]
    struct ProcessEntry32 {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [i8; MAX_PATH],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetCurrentProcessId() -> u32;
        fn SetProcessWorkingSetSize(
            process: *mut c_void,
            min_working_set: usize,
            max_working_set: usize,
        ) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
        fn Process32First(snapshot: *mut c_void, entry: *mut ProcessEntry32) -> i32;
        fn Process32Next(snapshot: *mut c_void, entry: *mut ProcessEntry32) -> i32;
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    fn invalid_handle() -> *mut c_void {
        -1isize as *mut c_void
    }

    /// (pid, parent_pid) for every process on the machine.
    fn process_pairs() -> Vec<(u32, u32)> {
        let mut pairs = Vec::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() || snapshot == invalid_handle() {
                return pairs;
            }
            let mut entry: ProcessEntry32 = std::mem::zeroed();
            entry.dw_size = std::mem::size_of::<ProcessEntry32>() as u32;
            if Process32First(snapshot, &mut entry) != 0 {
                loop {
                    pairs.push((entry.th32_process_id, entry.th32_parent_process_id));
                    if Process32Next(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snapshot);
        }
        pairs
    }

    /// WebView2 runs in its own process tree, so trimming only the host process
    /// leaves the renderer's memory resident. Walk our descendants instead.
    fn descendant_pids() -> Vec<u32> {
        let pairs = process_pairs();
        let mut found = Vec::new();
        let mut frontier = vec![unsafe { GetCurrentProcessId() }];
        while let Some(parent) = frontier.pop() {
            for (pid, ppid) in &pairs {
                if *ppid == parent && !found.contains(pid) {
                    found.push(*pid);
                    frontier.push(*pid);
                }
            }
        }
        found
    }

    pub fn release_working_set() {
        // (SIZE_T)-1 for both bounds is the documented way to ask Windows to
        // trim a process working set.
        unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            for pid in descendant_pids() {
                let handle = OpenProcess(PROCESS_SET_QUOTA, 0, pid);
                if handle.is_null() {
                    continue;
                }
                SetProcessWorkingSetSize(handle, usize::MAX, usize::MAX);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(not(windows))]
mod winmem {
    pub fn release_working_set() {}
}

/// Window creation itself emits resize events, and acting on those would
/// destroy the window before it is ever shown. Only resizes that happen while
/// this is set are treated as a real user minimize.
static WINDOW_READY: AtomicBool = AtomicBool::new(false);

const TRAY_ID: &str = "blackshark-tray";
const EVENT_BATTERY: &str = "battery://update";
const EVENT_OPEN_SETTINGS: &str = "ui://open-settings";
const DEFAULT_POLL_SECONDS: u64 = 60;
const MIN_POLL_SECONDS: u64 = 5;

struct TrayItems {
    header: MenuItem<Wry>,
    show: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct Shared {
    poll_seconds: AtomicU64,
    /// Set by `refresh_now`; the poll thread waits on `wake` instead of
    /// ticking, so it stays asleep for the whole interval.
    refresh_requested: Mutex<bool>,
    wake: Condvar,
    locale: Mutex<String>,
    last: Mutex<Option<BatteryReading>>,
    last_tooltip: Mutex<String>,
    tray_items: Mutex<Option<TrayItems>>,
}

impl Shared {
    fn new() -> Self {
        Self {
            poll_seconds: AtomicU64::new(DEFAULT_POLL_SECONDS),
            refresh_requested: Mutex::new(false),
            wake: Condvar::new(),
            locale: Mutex::new("tr".to_string()),
            last: Mutex::new(None),
            last_tooltip: Mutex::new(String::new()),
            tray_items: Mutex::new(None),
        }
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_seconds.load(Ordering::Relaxed).max(MIN_POLL_SECONDS))
    }

    /// Wakes the poll thread. `refresh` forces an immediate read; otherwise the
    /// thread just re-evaluates its deadline against the current interval.
    fn nudge(&self, refresh: bool) {
        if refresh {
            *self.refresh_requested.lock().unwrap() = true;
        }
        self.wake.notify_all();
    }
}

fn tooltip_for(locale: &str, reading: Option<&BatteryReading>) -> String {
    let tr = locale.starts_with("tr");
    match reading {
        Some(r) if r.ok => {
            let pct = r.percent.unwrap_or(0);
            let suffix = if r.charging {
                if tr {
                    "şarj oluyor"
                } else {
                    "charging"
                }
            } else if tr {
                "kablosuz"
            } else {
                "wireless"
            };
            format!("BlackShark V2: %{pct} ({suffix})")
        }
        _ => {
            if tr {
                "BlackShark V2: bağlı değil".to_string()
            } else {
                "BlackShark V2: offline".to_string()
            }
        }
    }
}

fn apply_tooltip(app: &AppHandle, shared: &Shared) {
    let locale = shared.locale.lock().unwrap().clone();
    let last = shared.last.lock().unwrap().clone();
    let tip = tooltip_for(&locale, last.as_ref());
    {
        // Every setter below hops to the main thread; skip the round-trip when
        // the text has not moved (the common case between two equal readings).
        let mut cached = shared.last_tooltip.lock().unwrap();
        if *cached == tip {
            return;
        }
        cached.clone_from(&tip);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(&tip));
    }
    if let Some(items) = shared.tray_items.lock().unwrap().as_ref() {
        let _ = items.header.set_text(&tip);
    }
}

fn main_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        WINDOW_READY.store(true, Ordering::Release);
        return;
    }

    // The webview was destroyed to free RAM, so rebuild it from the same
    // `tauri.conf.json` window definition instead of a hardcoded copy.
    let Some(config) = app.config().app.windows.first().cloned() else {
        return;
    };
    match WebviewWindowBuilder::from_config(app, &config).and_then(|builder| {
        builder
            .visible(true)
            .focused(true)
            .build()
    }) {
        Ok(window) => {
            let _ = window.set_focus();
            WINDOW_READY.store(true, Ordering::Release);
        }
        Err(err) => eprintln!("failed to recreate main window: {err}"),
    }
}

/// Destroys the webview so its process (and its RAM) goes away entirely. The
/// working set trim runs slightly later, once Windows has reaped the process.
fn drop_window_to_tray(window: &Window) {
    WINDOW_READY.store(false, Ordering::Release);
    let target = window.clone();
    let _ = window
        .app_handle()
        .run_on_main_thread(move || {
            let _ = target.destroy();
        });
    schedule_working_set_release();
}

fn schedule_working_set_release() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        winmem::release_working_set();
    });
}

fn poll_loop(app: AppHandle, shared: Arc<Shared>) {
    loop {
        let reading = battery::read_battery();
        *shared.last.lock().unwrap() = Some(reading.clone());
        apply_tooltip(&app, &shared);
        // Skip the IPC round-trip (and the webview repaint it triggers)
        // while the window is hidden in the tray.
        if main_window_visible(&app) {
            let _ = app.emit(EVENT_BATTERY, &reading);
        }

        wait_for_next_poll(&shared);
    }
}

/// Blocks until the poll interval has elapsed or a refresh is requested. The
/// deadline is re-derived from `poll_seconds` on every wake-up, so changing the
/// interval in the settings takes effect without waiting out the old one.
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

// Not `#[tauri::command]`: a plain sync command runs on the main thread, and a
// HID round-trip can block it for seconds. `async` puts it on the thread pool.
#[tauri::command(async)]
fn get_battery() -> BatteryReading {
    battery::read_battery()
}

#[tauri::command]
fn last_reading(state: State<'_, Arc<Shared>>) -> Option<BatteryReading> {
    state.last.lock().unwrap().clone()
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
    apply_tooltip(&app, &state);
}

#[tauri::command]
fn hide_to_tray(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        drop_window_to_tray(&window.as_ref().window_ref());
    }
}

/// Exposed so the frontend can trim RAM when it detects `visibilityState`
/// flipping to `hidden`.
#[tauri::command]
fn release_memory() {
    winmem::release_working_set();
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared = Arc::new(Shared::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            get_battery,
            last_reading,
            refresh_now,
            set_poll_seconds,
            apply_localization,
            hide_to_tray,
            release_memory,
            quit_app
        ])
        .on_window_event(|window, event| match event {
            // The close is allowed to proceed so the webview process dies; the
            // app itself is kept alive by prevent_exit in the run loop below.
            WindowEvent::Destroyed | WindowEvent::CloseRequested { .. } => {
                schedule_working_set_release();
            }
            // Tauri has no dedicated minimize event, so the minimized state is
            // detected from the resize that Windows sends alongside it.
            WindowEvent::Resized(_) => {
                if WINDOW_READY.load(Ordering::Acquire)
                    && window.is_minimized().unwrap_or(false)
                    && window.is_visible().unwrap_or(false)
                {
                    let _ = window.unminimize();
                    drop_window_to_tray(window);
                }
            }
            _ => {}
        })
        .setup(move |app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::MacosLauncher;
                app.handle().plugin(tauri_plugin_autostart::init(
                    MacosLauncher::LaunchAgent,
                    Some(vec!["--minimized"]),
                ))?;
            }

            let header = MenuItem::with_id(app, "header", "BlackShark V2", false, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Göster", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Ayarlar", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let head_separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[&header, &head_separator, &show, &settings, &separator, &quit],
            )?;

            let state = app.state::<Arc<Shared>>().inner().clone();
            *state.tray_items.lock().unwrap() = Some(TrayItems {
                header: header.clone(),
                show: show.clone(),
                settings: settings.clone(),
                quit: quit.clone(),
            });

            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("BlackShark V2")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "settings" => {
                        show_main_window(app);
                        let _ = app.emit(EVENT_OPEN_SETTINGS, ());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            let started_minimized = std::env::args().any(|a| a == "--minimized");
            if let Some(window) = app.get_webview_window("main") {
                if started_minimized {
                    // Autostart runs tray-only: tear the webview down at once.
                    let _ = window.destroy();
                    schedule_working_set_release();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                    WINDOW_READY.store(true, Ordering::Release);
                }
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || poll_loop(handle, state));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // Closing the last window would normally end the process. Only an
            // explicit app.exit(code) is allowed through.
            if let RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
