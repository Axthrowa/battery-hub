// Hide the console window on Windows (release AND debug GUI runs).
// Diagnostics still write to %LOCALAPPDATA%\Battery Hub\diagnostics.log.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    battery_hub_lib::run()
}
