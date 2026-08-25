//! Windows SetupAPI — any Bluetooth / AEP device that exposes a battery property.

use super::{Brand, DeviceReading};

#[cfg(windows)]
mod win {
    use super::{Brand, DeviceReading};
    use std::ffi::OsString;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;

    type Handle = *mut std::ffi::c_void;
    type DevInfo = Handle;

    const DIGCF_PRESENT: u32 = 0x0000_0002;
    const DIGCF_ALLCLASSES: u32 = 0x0000_0004;
    const INVALID_HANDLE_VALUE: isize = -1;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    const SPDRP_FRIENDLYNAME: u32 = 0x0000_000C;
    const SPDRP_DEVICEDESC: u32 = 0x0000_0000;
    const SPDRP_ENUMERATOR_NAME: u32 = 0x0000_0016;

    const DEVPROP_TYPE_BYTE: u32 = 0x0000_0003;
    const DEVPROP_TYPE_UINT32: u32 = 0x0000_0007;
    const DEVPROP_TYPE_BYTE_MASK: u32 = 0x0000_0FFF;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const GUID_DEVCLASS_BLUETOOTH: Guid = Guid {
        data1: 0xE0CBF06C,
        data2: 0xCD8B,
        data3: 0x4647,
        data4: [0xBB, 0x8A, 0x26, 0x3B, 0x43, 0xF0, 0xF9, 0x74],
    };

    #[repr(C)]
    struct SpDevinfoData {
        cb_size: u32,
        class_guid: Guid,
        dev_inst: u32,
        reserved: usize,
    }

    #[repr(C)]
    struct Devpropkey {
        fmtid: Guid,
        pid: u32,
    }

    const DEVPKEY_BLUETOOTH_BATTERY: Devpropkey = Devpropkey {
        fmtid: Guid {
            data1: 0x104E_A319,
            data2: 0x6EE2,
            data3: 0x4701,
            data4: [0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5],
        },
        pid: 2,
    };

    const DEVPKEY_AEP_BATTERY: Devpropkey = Devpropkey {
        fmtid: Guid {
            data1: 0xA92F_26CA,
            data2: 0x7735,
            data3: 0x4E1A,
            data4: [0x89, 0x70, 0x8B, 0x44, 0xF7, 0xE4, 0xF6, 0xC5],
        },
        pid: 2,
    };

    #[link(name = "setupapi")]
    unsafe extern "system" {
        fn SetupDiGetClassDevsW(
            class_guid: *const Guid,
            enumerator: *const u16,
            hwnd: Handle,
            flags: u32,
        ) -> DevInfo;
        fn SetupDiEnumDeviceInfo(set: DevInfo, index: u32, data: *mut SpDevinfoData) -> i32;
        fn SetupDiGetDeviceRegistryPropertyW(
            set: DevInfo,
            data: *const SpDevinfoData,
            property: u32,
            reg_type: *mut u32,
            buffer: *mut u8,
            buffer_size: u32,
            required: *mut u32,
        ) -> i32;
        fn SetupDiGetDevicePropertyW(
            set: DevInfo,
            data: *const SpDevinfoData,
            property_key: *const Devpropkey,
            property_type: *mut u32,
            buffer: *mut u8,
            buffer_size: u32,
            required: *mut u32,
            flags: u32,
        ) -> i32;
        fn SetupDiDestroyDeviceInfoList(set: DevInfo) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
    }

    struct DeviceSet(DevInfo);
    impl Drop for DeviceSet {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 as isize != INVALID_HANDLE_VALUE {
                unsafe {
                    SetupDiDestroyDeviceInfoList(self.0);
                }
            }
        }
    }

    fn read_reg_string(set: DevInfo, data: &SpDevinfoData, prop: u32) -> Option<String> {
        unsafe {
            let mut needed = 0u32;
            let mut reg_type = 0u32;
            SetupDiGetDeviceRegistryPropertyW(
                set,
                data,
                prop,
                &mut reg_type,
                ptr::null_mut(),
                0,
                &mut needed,
            );
            if GetLastError() != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
                return None;
            }
            let mut buf = vec![0u8; needed as usize];
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                data,
                prop,
                &mut reg_type,
                buf.as_mut_ptr(),
                needed,
                &mut needed,
            ) == 0
            {
                return None;
            }
            let wide: &[u16] = std::slice::from_raw_parts(
                buf.as_ptr() as *const u16,
                (buf.len() / 2).saturating_sub(1),
            );
            let s = OsString::from_wide(wide).to_string_lossy().trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
    }

    fn read_battery_prop(set: DevInfo, data: &SpDevinfoData, key: &Devpropkey) -> Option<u8> {
        unsafe {
            let mut prop_type = 0u32;
            let mut needed = 0u32;
            SetupDiGetDevicePropertyW(
                set,
                data,
                key,
                &mut prop_type,
                ptr::null_mut(),
                0,
                &mut needed,
                0,
            );
            if GetLastError() != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
                return None;
            }
            let mut buf = vec![0u8; needed as usize];
            if SetupDiGetDevicePropertyW(
                set,
                data,
                key,
                &mut prop_type,
                buf.as_mut_ptr(),
                needed,
                &mut needed,
                0,
            ) == 0
            {
                return None;
            }
            let kind = prop_type & DEVPROP_TYPE_BYTE_MASK;
            let value = if kind == DEVPROP_TYPE_BYTE {
                buf.first().copied()
            } else if kind == DEVPROP_TYPE_UINT32 && buf.len() >= 4 {
                Some(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u8)
            } else {
                buf.first().copied()
            }?;
            if value <= 100 {
                Some(value)
            } else {
                None
            }
        }
    }

    fn skip_name(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n.is_empty()
            || n.contains("enumerator")
            || n.contains("adapter")
            || n.contains("rfcomm")
            || n.contains("avrcp transport")
            || n.contains("microsoft bluetooth")
            || n.contains("tp-link bluetooth")
            || n.contains("generic bluetooth")
            || n.contains("radio")
    }

    /// Every Bluetooth-class device that exposes a Windows battery percentage.
    pub fn read_all() -> Vec<DeviceReading> {
        let mut out = Vec::new();
        unsafe {
            let set = SetupDiGetClassDevsW(
                &GUID_DEVCLASS_BLUETOOTH,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT,
            );
            let set = if set.is_null() || set as isize == INVALID_HANDLE_VALUE {
                SetupDiGetClassDevsW(
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    DIGCF_PRESENT | DIGCF_ALLCLASSES,
                )
            } else {
                set
            };
            if set.is_null() || set as isize == INVALID_HANDLE_VALUE {
                return out;
            }
            let set = DeviceSet(set);
            let mut index = 0u32;
            loop {
                let mut data: SpDevinfoData = zeroed();
                data.cb_size = size_of::<SpDevinfoData>() as u32;
                if SetupDiEnumDeviceInfo(set.0, index, &mut data) == 0 {
                    break;
                }
                index += 1;

                let name = read_reg_string(set.0, &data, SPDRP_FRIENDLYNAME)
                    .or_else(|| read_reg_string(set.0, &data, SPDRP_DEVICEDESC))
                    .unwrap_or_default();
                if skip_name(&name) {
                    continue;
                }
                let enumerator = read_reg_string(set.0, &data, SPDRP_ENUMERATOR_NAME)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                // Prefer BT / AEP nodes; still accept others with a battery prop.
                let btish = enumerator.contains("bth")
                    || enumerator.contains("bluetooth")
                    || enumerator.contains("aep");

                let pct = read_battery_prop(set.0, &data, &DEVPKEY_BLUETOOTH_BATTERY)
                    .or_else(|| read_battery_prop(set.0, &data, &DEVPKEY_AEP_BATTERY));
                let Some(percent) = pct else {
                    continue;
                };
                if !btish && percent == 0 {
                    continue;
                }

                let brand = Brand::classify("", &name);
                out.push(DeviceReading::ok(
                    brand,
                    name,
                    "Bluetooth",
                    percent,
                    false,
                ));
            }
        }
        out
    }
}

#[cfg(windows)]
pub fn read_all() -> Vec<DeviceReading> {
    win::read_all()
}

#[cfg(not(windows))]
pub fn read_all() -> Vec<DeviceReading> {
    Vec::new()
}
