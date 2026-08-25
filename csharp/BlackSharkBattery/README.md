# BlackShark Battery (C# / .NET 8)

Windows Forms sistem tepsisi uygulaması — penceresiz, Synapse gerekmez.

- **USB HID** (2.4 GHz dongle `1532:0565` / kablo `1532:056e`) — usage page `0xFF14`
- **Bluetooth** yedek: Windows PnP/HFP pil özelliği
- 60 sn’de bir güncelleme, tooltip: `Razer BlackShark: %85`
- Sağ tık → Şimdi yenile / Çıkış

## Gereksinim

[.NET 8 SDK](https://dotnet.microsoft.com/download/dotnet/8.0)

## Derleme (tek bağımsız .exe)

PowerShell:

```powershell
cd $env:USERPROFILE\blackshark-battery\csharp\BlackSharkBattery
.\publish.ps1
```

veya:

```powershell
dotnet publish -c Release -r win-x64 --self-contained true `
  -p:PublishSingleFile=true `
  -p:IncludeNativeLibrariesForSelfExtract=true `
  -p:EnableCompressionInSingleFile=true `
  -o .\publish
```

Çıktı: `publish\BlackSharkBattery.exe`

Küçük (SDK kurulu makineler için, framework-dependent):

```powershell
dotnet publish -c Release -r win-x64 --self-contained false `
  -p:PublishSingleFile=true -o .\publish-fd
```

## Çalıştırma

`BlackSharkBattery.exe` dosyasına çift tıklayın. Görev çubuğu taşmasında (`^`) simgeyi görün.

## Not

Synapse HID’i kilitliyorsa kapatın. Kulaklık 2.4 GHz modunda ve açık olsun.
