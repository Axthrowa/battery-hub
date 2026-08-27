//! Windows WinRT Bluetooth LE — GATT Battery Service (0x180F) / Battery Level (0x2A19).

use super::{Brand, DeviceReading};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BleBatteryInfo {
    pub ok: bool,
    pub name: String,
    pub percent: Option<u8>,
    pub device_id: String,
    pub error: Option<String>,
}

#[cfg(windows)]
mod win {
    use super::{BleBatteryInfo, Brand, DeviceReading};
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids,
    };
    use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Storage::Streams::DataReader;
    use windows::core::HSTRING;

    fn is_target_name(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        crate::devices::hid::SOUNDCORE_NAME_HINTS
            .iter()
            .any(|h| n.contains(h))
            || n.contains("headset")
            || n.contains("earbud")
            || n.contains("headphone")
    }

    fn read_battery_level(ble: &BluetoothLEDevice) -> windows::core::Result<Option<u8>> {
        let services = ble
            .GetGattServicesForUuidWithCacheModeAsync(
                GattServiceUuids::Battery()?,
                BluetoothCacheMode::Uncached,
            )?
            .get()?;

        if services.Status()? != GattCommunicationStatus::Success {
            return Ok(None);
        }

        let list = services.Services()?;
        if list.Size()? == 0 {
            return Ok(None);
        }

        let service = list.GetAt(0)?;
        let chars = service
            .GetCharacteristicsForUuidWithCacheModeAsync(
                GattCharacteristicUuids::BatteryLevel()?,
                BluetoothCacheMode::Uncached,
            )?
            .get()?;

        if chars.Status()? != GattCommunicationStatus::Success {
            return Ok(None);
        }

        let characteristics = chars.Characteristics()?;
        if characteristics.Size()? == 0 {
            return Ok(None);
        }

        let characteristic = characteristics.GetAt(0)?;
        let read = characteristic.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?.get()?;
        if read.Status()? != GattCommunicationStatus::Success {
            return Ok(None);
        }

        let buffer = read.Value()?;
        let reader = DataReader::FromBuffer(&buffer)?;
        let level = reader.ReadByte()?;
        if (1..=100).contains(&level) {
            Ok(Some(level))
        } else if level == 0 {
            Ok(Some(0))
        } else {
            Ok(None)
        }
    }

    /// Scan paired/nearby BLE devices and read GATT Battery Service (0x180F).
    pub fn scan_ble_batteries() -> Vec<BleBatteryInfo> {
        let mut out = Vec::new();
        let result = (|| -> windows::core::Result<()> {
            let selector = BluetoothLEDevice::GetDeviceSelector()?;
            let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.get()?;
            let count = devices.Size()?;

            for i in 0..count {
                let info = devices.GetAt(i)?;
                let name = info.Name()?.to_string();
                let id = info.Id()?.to_string();

                // Try every paired BLE device — Battery Hub is multi-brand.
                let prefer = is_target_name(&name);

                let ble = match BluetoothLEDevice::FromIdAsync(&HSTRING::from(id.as_str()))?.get() {
                    Ok(d) => d,
                    Err(e) => {
                        if prefer {
                            out.push(BleBatteryInfo {
                                ok: false,
                                name: name.clone(),
                                percent: None,
                                device_id: id.clone(),
                                error: Some(format!("FromIdAsync failed: {e}")),
                            });
                        }
                        continue;
                    }
                };

                // Request shared access so we don't kick other apps.
                let _ = ble.RequestAccessAsync()?.get();

                match read_battery_level(&ble) {
                    Ok(Some(pct)) => {
                        out.push(BleBatteryInfo {
                            ok: true,
                            name,
                            percent: Some(pct),
                            device_id: id,
                            error: None,
                        });
                    }
                    Ok(None) => {
                        if prefer {
                            out.push(BleBatteryInfo {
                                ok: false,
                                name,
                                percent: None,
                                device_id: id,
                                error: Some(
                                    "GATT Battery Service (0x180F) unavailable or empty".into(),
                                ),
                            });
                        }
                    }
                    Err(e) => {
                        if prefer {
                            out.push(BleBatteryInfo {
                                ok: false,
                                name,
                                percent: None,
                                device_id: id,
                                error: Some(format!("GATT read failed: {e}")),
                            });
                        }
                    }
                }

                // Close device when done.
                let _ = ble.Close();
            }
            Ok(())
        })();

        if let Err(e) = result {
            out.push(BleBatteryInfo {
                ok: false,
                name: String::new(),
                percent: None,
                device_id: String::new(),
                error: Some(format!("BLE enumeration failed: {e}")),
            });
        }

        // Sort: Soundcore-like names first, then successes.
        out.sort_by(|a, b| {
            let ap = is_target_name(&a.name) as u8;
            let bp = is_target_name(&b.name) as u8;
            bp.cmp(&ap).then_with(|| b.ok.cmp(&a.ok))
        });

        out
    }

    /// Every BLE device with a readable GATT Battery Service.
    pub fn read_all_devices() -> Vec<DeviceReading> {
        let mut out = Vec::new();
        for r in scan_ble_batteries() {
            if r.ok {
                if let Some(pct) = r.percent {
                    let name = if r.name.trim().is_empty() {
                        "Bluetooth LE Device".into()
                    } else {
                        r.name
                    };
                    let brand = Brand::classify("", &name);
                    out.push(DeviceReading::ok(
                        brand,
                        name,
                        "Bluetooth LE",
                        pct,
                        false,
                    ));
                }
            }
        }
        out
    }
}

#[cfg(windows)]
pub fn read_all_devices() -> Vec<DeviceReading> {
    win::read_all_devices()
}

#[cfg(not(windows))]
pub fn read_all_devices() -> Vec<DeviceReading> {
    Vec::new()
}
