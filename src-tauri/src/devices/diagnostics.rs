//! Hardware diagnostics — HID/BT dumps to a log file (no console window).
//!
//! Set env `BATTERY_HUB_DEBUG=1` to also AllocConsole for live terminal output.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The tray app runs for days — keep the diagnostics log from growing forever.
const LOG_MAX_BYTES: u64 = 1024 * 1024;

static CONSOLE_READY: AtomicBool = AtomicBool::new(false);
static SCAN_COUNT: AtomicU64 = AtomicU64::new(0);

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{}.{:03}", d.as_secs(), d.subsec_millis()))
        .unwrap_or_else(|_| "?".into())
}

fn log_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)?;
    let dir = base.join("Battery Hub");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("diagnostics.log"))
}

fn debug_console_enabled() -> bool {
    matches!(
        std::env::var("BATTERY_HUB_DEBUG").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}

fn emit(line: &str) {
    if debug_console_enabled() {
        println!("{line}");
    }
    if let Some(path) = log_path() {
        let oversized = std::fs::metadata(&path)
            .map(|meta| meta.len() > LOG_MAX_BYTES)
            .unwrap_or(false);
        if oversized {
            let _ = std::fs::write(&path, b"[log] truncated (1 MiB cap)\n");
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Public wrapper so other modules / the poll loop can append to the same log.
pub fn emit_line(line: &str) {
    ensure_debug_console();
    emit(line);
}

/// Only allocates a visible console when `BATTERY_HUB_DEBUG=1`.
pub fn ensure_debug_console() {
    if !debug_console_enabled() {
        return;
    }
    if CONSOLE_READY.swap(true, Ordering::AcqRel) {
        return;
    }
    #[cfg(windows)]
    {
        type Bool = i32;
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(dw_process_id: u32) -> Bool;
            fn AllocConsole() -> Bool;
        }
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                let _ = AllocConsole();
            }
        }
    }
    emit(&format!(
        "[Battery Hub] diagnostics console ready @ {}",
        timestamp()
    ));
    if let Some(path) = log_path() {
        emit(&format!(
            "[Battery Hub] also writing to {}",
            path.display()
        ));
    }
}

/// Dump EVERY HID device on the bus (not filtered by brand VID).
pub fn dump_all_hid_devices() {
    ensure_debug_console();
    emit("========== HID SCAN DUMP (ALL DEVICES) ==========");
    emit(&format!("time={}", timestamp()));
    emit("Copy VID/PID of your Logitech / Ajazz dongle into src-tauri/src/devices/hid.rs");

    match hidapi::HidApi::new() {
        Ok(api) => {
            let mut n = 0u32;
            for d in api.device_list() {
                n += 1;
                let manufacturer = d.manufacturer_string().unwrap_or("?");
                let product = d.product_string().unwrap_or("?");
                let path = d.path().to_string_lossy();
                emit(&format!(
                    "  [{n:03}] VID=0x{:04X} PID=0x{:04X} usage_page=0x{:04X} usage=0x{:04X} | {manufacturer} | {product} | {path}",
                    d.vendor_id(),
                    d.product_id(),
                    d.usage_page(),
                    d.usage(),
                ));
            }
            if n == 0 {
                emit("  (no HID devices enumerated)");
            } else {
                emit(&format!("  total HID interfaces: {n}"));
            }
        }
        Err(e) => emit(&format!("  HID init failed: {e}")),
    }
    emit("======== END HID SCAN DUMP ========");
}

/// Dump every Windows Bluetooth class device + battery property probe.
pub fn dump_bluetooth_devices() {
    ensure_debug_console();
    emit("========== BLUETOOTH SCAN DUMP ==========");
    emit(&format!("time={}", timestamp()));

    #[cfg(windows)]
    {
        for line in crate::devices::soundcore::diagnose_all() {
            emit(&line);
        }
    }
    #[cfg(not(windows))]
    {
        emit("  Bluetooth diagnose requires Windows.");
    }

    emit("======== END BLUETOOTH SCAN DUMP ========");
}

/// Called at the top of every poll loop iteration.
pub fn run_poll_diagnostics() {
    let n = SCAN_COUNT.fetch_add(1, Ordering::Relaxed);
    if n == 0 || n % 10 == 0 {
        dump_all_hid_devices();
        dump_bluetooth_devices();
    } else {
        emit(&format!(
            "[diag] poll #{n} (full HID/BT dump every 10 polls — see diagnostics.log)"
        ));
    }
}
