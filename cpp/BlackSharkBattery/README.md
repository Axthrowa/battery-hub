# BlackShark Battery (Win32 + GDI+)

Pure Win32 tray app. HID/Bluetooth reading unchanged. UI is async + GDI+.

## Features

- Background `std::thread` battery polling (UI never blocks)
- GDI+ anti-aliased battery icon (green / orange / red + lightning when charging)
- Dark owner-drawn context menu
- Tooltip: `BlackShark V2: %NN`
- Menu: header, Refresh Now, Run at Startup, Exit

## Build (MSVC)

```bat
cd cpp\BlackSharkBattery
build.bat
```

Output:

```text
publish\BlackSharkBattery.exe
```

## Run

Double-click the exe. Right-click tray icon for menu.
