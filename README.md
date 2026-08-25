# Battery Hub

Multi-brand wireless battery monitor for Windows (Tauri 2 + React).

## How devices are found

Every poll runs these readers in parallel and merges them (duplicates collapse,
a vendor reading beats a generic one):

| Reader | Covers |
|--------|--------|
| `devices/ble_gatt.rs` | Any paired Bluetooth LE device exposing GATT Battery Service `0x180F` / Battery Level `0x2A19` |
| `devices/windows_battery.rs` | Classic Bluetooth / AEP devices via `DEVPKEY_Bluetooth_Battery` (SetupAPI) |
| `devices/hid_battery.rs` | Any HID device whose report descriptor declares `Battery Strength` (page `0x06`) or Battery System (page `0x85`) |
| `devices/razer.rs` | Razer BlackShark V2 HyperSpeed — vendor HID over the HyperSpeed dongle |
| `devices/logitech.rs` | Logitech HID++ `0x1000` / `0x1001` on the USB receiver |
| `devices/ajazz.rs` | Ajazz 2.4 GHz custom HID |
| `devices/soundcore.rs` | Soundcore / Anker over Bluetooth |

`devices/hid_descriptor.rs` is the HID report-descriptor parser behind the
generic path — it locates the exact bits holding the state of charge, so no
byte guessing is involved. It is pure logic and unit tested:

```bash
cargo test -p battery-hub hid_descriptor
```

## Adding a device the readers do not know

Hardware that exposes no standard battery field still tends to publish the
state of charge as a plain byte in a vendor feature report. **Cihaz Ekle** in
the UI samples every HID report twice, keeps the bytes that stayed put and look
like a percentage, and offers them. The user recognises the real value, and only
that exact location — `VID:PID:usage page:report ID:byte offset` — is stored in
`%LOCALAPPDATA%\Battery Hub\devices.json` and read back on every poll.

Guessing is confined to this confirmed path: the automatic readers never invent
a percentage. To see what a scan finds without opening the UI:

```powershell
battery-hub.exe --scan-devices    # writes the candidates to diagnostics.log
```

## Tray behaviour

Closing the window **destroys WebView2** (no `hide()`). The Rust host stays in
the system tray with near-zero UI RAM; tray → **Göster** recreates the window.
The console window is suppressed via `windows_subsystem = "windows"` in
`src-tauri/src/main.rs`.

## Vendor IDs

`src-tauri/src/devices/hid.rs` holds the VID/PID table for the vendor-specific
readers. The generic readers need no table.

## Icons

`src-tauri/icons/` is generated from `app-icon.png` (1024×1024, transparent):

```bash
npm run icon                    # tauri icon app-icon.png (full set)
python scripts/make-icons.py    # then rebuild the Windows sizes
```

The second step matters: the artwork is a tall battery, so a plain square
downscale leaves a sliver at 16x16 — the size Task Manager, the tray and the
title bar use. `make-icons.py` gives those sizes their own square crop and
keeps the whole device for 48 px and above.

## Develop

```bash
npm install
npm run tauri:dev
```

## Build (Windows)

```bash
npm install
npm run tauri:build
```

| Artifact | Path |
|----------|------|
| Portable `.exe` | `src-tauri/target/release/battery-hub.exe` |
| NSIS setup | `src-tauri/target/release/bundle/nsis/Battery Hub_0.1.0_x64-setup.exe` |

Add `"msi"` to `bundle.targets` in `src-tauri/tauri.conf.json` for a WiX MSI as
well. Build on **Windows** — WSL alone cannot produce a native WebView2 binary.

## Code signing

The certificate is machine-specific, so it is kept out of `tauri.conf.json`.
Copy the template and fill in your own thumbprint:

```bash
cp src-tauri/tauri.local.conf.json.example src-tauri/tauri.local.conf.json
```

```bash
npm run tauri:build:signed   # merges the local config, signs the installer
npm run sign                 # signs the portable battery-hub.exe
```

`npm run tauri:build` still produces an unsigned build, so cloning and building
this repo works without any certificate. The extra `npm run sign` pass exists
because Tauri signs the executable before packing it into the installer and then
restores the unsigned original on disk.

A self-signed certificate is only trusted where it has been imported into
`Cert:\CurrentUser\Root`. It removes the "Unknown publisher" prompt on that
machine; it does **not** clear Microsoft Defender SmartScreen elsewhere, because
SmartScreen scores publisher reputation, not the mere presence of a signature.
For distribution, use an OV/EV certificate from a public CA — an EV certificate
is what clears SmartScreen from day one.

## Optional: restart when the receiver is re-plugged

```powershell
scripts\device-trigger\Install-DeviceTrigger.ps1
```
