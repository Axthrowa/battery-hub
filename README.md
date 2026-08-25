# BlackShark Battery

Razer BlackShark V2 HyperSpeed pil yüzdesi — Windows (Synapse yok).

## Tauri + React (önerilen UI)

Ayarlar modalı, TR/EN, yenileme süresi, kalıcı ayarlar:

```powershell
cd blackshark-desktop
npm install
npm run tauri:dev
npm run tauri:build
```

Ayrıntılar: [blackshark-desktop/README.md](blackshark-desktop/README.md)

## C++ (Win32 tepsi)

`cpp\BlackSharkBattery\publish\BlackSharkBattery.exe` — `cpp\BlackSharkBattery\build.bat`

## C# / Python

`csharp\` ve kök Python uygulaması da mevcut.
