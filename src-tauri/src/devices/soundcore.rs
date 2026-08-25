//! Soundcore Select 4 Go — Windows Bluetooth presence + SPP/COM battery probe.
//!
//! Windows does not expose a DEVPROP battery for this classic-BT speaker.
//! Battery is queried over the paired RFCOMM serial port (COM*) whose PnP
//! instance contains the Soundcore MAC / Anker VID 0x05D6.

use super::{Brand, DeviceReading};

#[cfg(windows)]
mod winbt {
    use super::{Brand, DeviceReading};
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    type Handle = *mut std::ffi::c_void;
    type DevInfo = Handle;

    const DIGCF_PRESENT: u32 = 0x0000_0002;
    const DIGCF_ALLCLASSES: u32 = 0x0000_0004;
    const INVALID_HANDLE_VALUE: isize = -1;
    const ERROR_NO_MORE_ITEMS: u32 = 259;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE: Handle = -1isize as Handle;

    const SPDRP_FRIENDLYNAME: u32 = 0x0000_000C;
    const SPDRP_DEVICEDESC: u32 = 0x0000_0000;
    const SPDRP_HARDWAREID: u32 = 0x0000_0001;
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

    #[repr(C)]
    struct Dcb {
        dc_length: u32,
        baud_rate: u32,
        flags: u32,
        w_reserved: u16,
        xon_lim: u16,
        xoff_lim: u16,
        byte_size: u8,
        parity: u8,
        stop_bits: u8,
        xon_char: i8,
        xoff_char: i8,
        error_char: i8,
        eof_char: i8,
        evt_char: i8,
        w_reserved1: u16,
    }

    #[repr(C)]
    struct CommTimeouts {
        read_interval_timeout: u32,
        read_total_timeout_multiplier: u32,
        read_total_timeout_constant: u32,
        write_total_timeout_multiplier: u32,
        write_total_timeout_constant: u32,
    }

    #[link(name = "setupapi")]
    extern "system" {
        fn SetupDiGetClassDevsW(
            class_guid: *const Guid,
            enumerator: *const u16,
            hwnd: Handle,
            flags: u32,
        ) -> DevInfo;
        fn SetupDiEnumDeviceInfo(
            device_info_set: DevInfo,
            member_index: u32,
            device_info_data: *mut SpDevinfoData,
        ) -> i32;
        fn SetupDiGetDeviceRegistryPropertyW(
            device_info_set: DevInfo,
            device_info_data: *const SpDevinfoData,
            property: u32,
            property_reg_data_type: *mut u32,
            property_buffer: *mut u8,
            property_buffer_size: u32,
            required_size: *mut u32,
        ) -> i32;
        fn SetupDiGetDevicePropertyW(
            device_info_set: DevInfo,
            device_info_data: *const SpDevinfoData,
            property_key: *const Devpropkey,
            property_type: *mut u32,
            property_buffer: *mut u8,
            property_buffer_size: u32,
            required_size: *mut u32,
            flags: u32,
        ) -> i32;
        fn SetupDiDestroyDeviceInfoList(device_info_set: DevInfo) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLastError() -> u32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: Handle,
            creation: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetCommState(handle: Handle, dcb: *mut Dcb) -> i32;
        fn SetCommState(handle: Handle, dcb: *const Dcb) -> i32;
        fn SetCommTimeouts(handle: Handle, timeouts: *const CommTimeouts) -> i32;
        fn PurgeComm(handle: Handle, flags: u32) -> i32;
    }

    fn invalid(handle: DevInfo) -> bool {
        handle.is_null() || handle as isize == INVALID_HANDLE_VALUE
    }

    fn wide_to_string(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        OsString::from_wide(&buf[..len])
            .to_string_lossy()
            .into_owned()
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn registry_string(set: DevInfo, data: &SpDevinfoData, prop: u32) -> Option<String> {
        let mut needed = 0u32;
        unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set,
                data,
                prop,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut needed,
            );
            if GetLastError() != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
                return None;
            }
            let mut buf = vec![0u8; needed as usize];
            let mut reg_type = 0u32;
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                data,
                prop,
                &mut reg_type,
                buf.as_mut_ptr(),
                needed,
                ptr::null_mut(),
            ) == 0
            {
                return None;
            }
            let words = buf.len() / 2;
            let mut wide = vec![0u16; words];
            for i in 0..words {
                wide[i] = u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]);
            }
            Some(wide_to_string(&wide))
        }
    }

    fn read_battery_prop(set: DevInfo, data: &SpDevinfoData, key: &Devpropkey) -> Option<u8> {
        let mut prop_type = 0u32;
        let mut needed = 0u32;
        unsafe {
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
            if needed == 0 {
                return None;
            }
            let mut buf = vec![0u8; needed.max(4) as usize];
            if SetupDiGetDevicePropertyW(
                set,
                data,
                key,
                &mut prop_type,
                buf.as_mut_ptr(),
                buf.len() as u32,
                ptr::null_mut(),
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
            if value == 0 || value > 100 {
                return None;
            }
            Some(value)
        }
    }

    fn is_soundcore_name(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        crate::devices::hid::SOUNDCORE_NAME_HINTS
            .iter()
            .any(|hint| n.contains(hint))
    }

    struct DeviceSet(DevInfo);
    impl Drop for DeviceSet {
        fn drop(&mut self) {
            if !invalid(self.0) {
                unsafe {
                    SetupDiDestroyDeviceInfoList(self.0);
                }
            }
        }
    }

    /// Find COMx whose PnP id mentions the Soundcore MAC / Anker BT VID.
    fn find_soundcore_com_port() -> Option<String> {
        unsafe {
            let set = SetupDiGetClassDevsW(
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT | DIGCF_ALLCLASSES,
            );
            if invalid(set) {
                return None;
            }
            let set = DeviceSet(set);
            let mut index = 0u32;
            loop {
                let mut data: SpDevinfoData = zeroed();
                data.cb_size = size_of::<SpDevinfoData>() as u32;
                if SetupDiEnumDeviceInfo(set.0, index, &mut data) == 0 {
                    if GetLastError() == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    index += 1;
                    continue;
                }
                index += 1;
                let friendly = registry_string(set.0, &data, SPDRP_FRIENDLYNAME).unwrap_or_default();
                let hwid = registry_string(set.0, &data, SPDRP_HARDWAREID).unwrap_or_default();
                let blob = format!("{friendly} {hwid}").to_ascii_lowercase();
                if !(blob.contains("f49d8a47d51e")
                    || (blob.contains("05d6") && blob.contains("bthenum") && blob.contains("00001101")))
                {
                    continue;
                }
                // FriendlyName like: "... (COM3)"
                if let Some(start) = friendly.find("(COM") {
                    let rest = &friendly[start + 1..];
                    if let Some(end) = rest.find(')') {
                        return Some(rest[..end].to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_battery_from_spp(buf: &[u8]) -> Option<u8> {
        if buf.is_empty() {
            return None;
        }
        // Prefer Soundcore 08 EE framed replies: scan after header for 1..=100.
        if buf.len() >= 10 && buf[0] == 0x08 && buf[1] == 0xEE {
            for &b in buf.iter().skip(9).take(16) {
                if (1..=100).contains(&b) {
                    return Some(b);
                }
            }
            // Some speakers report 0..5 bars.
            if let Some(&bars) = buf.get(10) {
                if bars <= 5 {
                    return Some([0, 20, 40, 60, 80, 100][bars as usize]);
                }
            }
        }
        for &b in buf.iter().take(24) {
            if (5..=100).contains(&b) {
                return Some(b);
            }
        }
        None
    }

    /// An SPP probe costs seconds and never succeeds on devices that do not
    /// speak Anker's RFCOMM protocol, so a failure backs off instead of being
    /// paid again on every poll.
    const SPP_COOLDOWN_MS: u64 = 300_000;
    const SPP_BUDGET: Duration = Duration::from_secs(3);
    static SPP_FAILED_AT_MS: AtomicU64 = AtomicU64::new(0);

    fn spp_cooling_down() -> bool {
        let failed_at = SPP_FAILED_AT_MS.load(Ordering::Relaxed);
        failed_at > 0 && crate::devices::now_ms().saturating_sub(failed_at) < SPP_COOLDOWN_MS
    }

    fn spp_query_battery(port: &str) -> Option<u8> {
        let path = format!(r"\\.\{port}");
        let wide = to_wide(&path);
        let frames: [&[u8]; 6] = [
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x01],
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x08],
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x09],
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x18],
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x05],
            &[0x08, 0xEE, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0B, 0x00, 0x01, 0x00],
        ];
        let deadline = Instant::now() + SPP_BUDGET;
        for baud in [9600u32, 115_200, 57_600] {
            if Instant::now() >= deadline {
                break;
            }
            unsafe {
                let handle = CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    ptr::null_mut(),
                );
                if handle.is_null() || handle == INVALID_HANDLE {
                    continue;
                }
                let mut dcb: Dcb = zeroed();
                dcb.dc_length = size_of::<Dcb>() as u32;
                if GetCommState(handle, &mut dcb) == 0 {
                    CloseHandle(handle);
                    continue;
                }
                dcb.baud_rate = baud;
                dcb.byte_size = 8;
                dcb.parity = 0;
                dcb.stop_bits = 0;
                let _ = SetCommState(handle, &dcb);
                let timeouts = CommTimeouts {
                    read_interval_timeout: 50,
                    read_total_timeout_multiplier: 0,
                    read_total_timeout_constant: 400,
                    write_total_timeout_multiplier: 0,
                    write_total_timeout_constant: 400,
                };
                let _ = SetCommTimeouts(handle, &timeouts);
                // PURGE_RXCLEAR | PURGE_TXCLEAR
                let _ = PurgeComm(handle, 0x0008 | 0x0004);

                // Wrap handle in File for Write/Read — duplicate ownership carefully.
                let mut file = std::fs::File::from_raw_handle(handle as RawHandle);
                for frame in frames {
                    if Instant::now() >= deadline {
                        break;
                    }
                    let _ = file.write_all(frame);
                    let _ = file.flush();
                    std::thread::sleep(Duration::from_millis(350));
                    let mut buf = [0u8; 128];
                    if let Ok(n) = file.read(&mut buf) {
                        if n > 0 {
                            if let Some(pct) = extract_battery_from_spp(&buf[..n]) {
                                // File drop closes handle.
                                drop(file);
                                return Some(pct);
                            }
                        }
                    }
                }
                drop(file);
            }
        }
        None
    }

    pub fn read() -> DeviceReading {
        // GATT 0x180F is already covered by the generic BLE reader, which runs
        // in its own poll thread — scanning again here would double the work.
        // Windows battery property first, then SPP.
        if let Some(reading) = read_via_setupapi() {
            if reading.ok {
                return reading;
            }
            // 2) Device present — try SPP/COM.
            if let Some(port) = find_soundcore_com_port() {
                if spp_cooling_down() {
                    return DeviceReading::failed(
                        Brand::soundcore(),
                        reading.product,
                        "Bluetooth",
                        format!("Found on {port}; SPP battery probe backing off after a failure."),
                        true,
                    );
                }
                if let Some(pct) = spp_query_battery(&port) {
                    SPP_FAILED_AT_MS.store(0, Ordering::Relaxed);
                    return DeviceReading::ok(
                        Brand::soundcore(),
                        reading.product,
                        "Bluetooth SPP",
                        pct,
                        false,
                    );
                }
                SPP_FAILED_AT_MS.store(crate::devices::now_ms(), Ordering::Relaxed);
                return DeviceReading::failed(
                    Brand::soundcore(),
                    reading.product,
                    "Bluetooth",
                    format!(
                        "Found on {port}; GATT/SPP battery unavailable (classic BT may not expose 0x180F)."
                    ),
                    true,
                );
            }
            return reading;
        }

        DeviceReading::failed(
            Brand::soundcore(),
            "soundcore Select 4 Go",
            "Bluetooth",
            "No connected Soundcore Bluetooth device found.",
            false,
        )
    }

    fn read_via_setupapi() -> Option<DeviceReading> {
        unsafe {
            let set = SetupDiGetClassDevsW(
                &GUID_DEVCLASS_BLUETOOTH,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT,
            );
            let set = if invalid(set) {
                SetupDiGetClassDevsW(
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    DIGCF_PRESENT | DIGCF_ALLCLASSES,
                )
            } else {
                set
            };
            if invalid(set) {
                return None;
            }
            Some(scan(DeviceSet(set)))
        }
    }

    fn scan(set: DeviceSet) -> DeviceReading {
        let mut index = 0u32;
        let mut saw_named = false;
        let mut last_name = String::from("soundcore Select 4 Go");

        loop {
            let mut data: SpDevinfoData = unsafe { zeroed() };
            data.cb_size = size_of::<SpDevinfoData>() as u32;
            let ok = unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                if err == ERROR_NO_MORE_ITEMS {
                    break;
                }
                index += 1;
                continue;
            }
            index += 1;

            let friendly = registry_string(set.0, &data, SPDRP_FRIENDLYNAME)
                .or_else(|| registry_string(set.0, &data, SPDRP_DEVICEDESC))
                .unwrap_or_default();
            let hwid = registry_string(set.0, &data, SPDRP_HARDWAREID).unwrap_or_default();
            let enumerator =
                registry_string(set.0, &data, SPDRP_ENUMERATOR_NAME).unwrap_or_default();
            let blob = format!("{friendly} {hwid} {enumerator}").to_ascii_lowercase();

            let name_ok = (!friendly.is_empty() && is_soundcore_name(&friendly))
                || blob.contains("soundcore")
                || blob.contains("select 4 go")
                || blob.contains("f49d8a47d51e");
            if !name_ok {
                continue;
            }

            saw_named = true;
            if !friendly.is_empty() && !friendly.to_ascii_lowercase().contains("avrcp") {
                last_name = friendly.trim().to_string();
            }

            if let Some(pct) = read_battery_prop(set.0, &data, &DEVPKEY_BLUETOOTH_BATTERY)
                .or_else(|| read_battery_prop(set.0, &data, &DEVPKEY_AEP_BATTERY))
            {
                return DeviceReading::ok(
                    Brand::soundcore(),
                    &last_name,
                    "Bluetooth",
                    pct,
                    false,
                );
            }
        }

        if saw_named {
            DeviceReading::failed(
                Brand::soundcore(),
                &last_name,
                "Bluetooth",
                "soundcore Select 4 Go found; Windows battery property unavailable — trying SPP.",
                true,
            )
        } else {
            DeviceReading::failed(
                Brand::soundcore(),
                "soundcore Select 4 Go",
                "Bluetooth",
                "No connected Soundcore Bluetooth device found.",
                false,
            )
        }
    }

    pub fn diagnose_all() -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(port) = find_soundcore_com_port() {
            lines.push(format!("  [BT] Soundcore SPP serial port: {port}"));
        } else {
            lines.push("  [BT] Soundcore SPP serial port: (not found)".into());
        }
        unsafe {
            let set = SetupDiGetClassDevsW(
                &GUID_DEVCLASS_BLUETOOTH,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT,
            );
            let set = if invalid(set) {
                SetupDiGetClassDevsW(
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    DIGCF_PRESENT | DIGCF_ALLCLASSES,
                )
            } else {
                set
            };
            if invalid(set) {
                lines.push("  [BT] SetupDiGetClassDevsW failed".into());
                return lines;
            }
            let set = DeviceSet(set);
            let mut index = 0u32;
            let mut n = 0u32;
            loop {
                let mut data: SpDevinfoData = zeroed();
                data.cb_size = size_of::<SpDevinfoData>() as u32;
                let ok = SetupDiEnumDeviceInfo(set.0, index, &mut data);
                if ok == 0 {
                    if GetLastError() == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    index += 1;
                    continue;
                }
                index += 1;
                let friendly = registry_string(set.0, &data, SPDRP_FRIENDLYNAME)
                    .or_else(|| registry_string(set.0, &data, SPDRP_DEVICEDESC))
                    .unwrap_or_else(|| "(no name)".into());
                let hwid = registry_string(set.0, &data, SPDRP_HARDWAREID).unwrap_or_default();
                let enumerator =
                    registry_string(set.0, &data, SPDRP_ENUMERATOR_NAME).unwrap_or_default();
                let blob = format!("{friendly} {hwid} {enumerator}").to_ascii_lowercase();
                let keep = blob.contains("bth")
                    || blob.contains("bluetooth")
                    || blob.contains("soundcore")
                    || blob.contains("select")
                    || blob.contains("f49d8a")
                    || blob.contains("05d6");
                if !keep {
                    continue;
                }
                n += 1;
                let bat = read_battery_prop(set.0, &data, &DEVPKEY_BLUETOOTH_BATTERY)
                    .or_else(|| read_battery_prop(set.0, &data, &DEVPKEY_AEP_BATTERY));
                let bat_s = bat
                    .map(|p| format!("battery={p}%"))
                    .unwrap_or_else(|| "battery=(none)".into());
                let match_hint = if is_soundcore_name(&friendly) || blob.contains("soundcore") {
                    " ★ SOUNDCORE-HINT MATCH"
                } else {
                    ""
                };
                lines.push(format!(
                    "  [BT {n:03}] name=\"{friendly}\" | {bat_s} | enum={enumerator} | hwid={hwid}{match_hint}"
                ));
            }
            lines.push(format!("  [BT] total listed: {n}"));
        }
        lines
    }

    #[allow(dead_code)]
    pub fn present() -> bool {
        matches!(read(), r if r.present || r.ok)
    }
}

#[cfg(windows)]
pub fn read() -> DeviceReading {
    winbt::read()
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn present() -> bool {
    winbt::present()
}

#[cfg(windows)]
pub fn diagnose_all() -> Vec<String> {
    winbt::diagnose_all()
}

#[cfg(not(windows))]
pub fn read() -> DeviceReading {
    DeviceReading::failed(
        Brand::soundcore(),
        "Soundcore",
        "Bluetooth",
        "Soundcore Bluetooth battery requires Windows.",
        false,
    )
}

#[cfg(not(windows))]
pub fn present() -> bool {
    false
}

#[cfg(not(windows))]
pub fn diagnose_all() -> Vec<String> {
    vec!["  [BT] diagnose_all requires Windows".into()]
}
