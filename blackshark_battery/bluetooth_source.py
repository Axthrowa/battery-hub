"""Windows Bluetooth battery reader (classic HFP/PnP + BLE GATT)."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
from dataclasses import dataclass, field
from typing import Any

NAME_HINTS = (
    "blackshark v2 hs",
    "blackshark v2 hyperspeed",
    "razer blackshark v2",
    "blackshark v2 hs bt",
)

# Undocumented PnP key Windows fills from HFP battery indicators.
# Same property used by several open-source Windows tray monitors.
BTH_BATTERY_FMTID = (0x104EA319, 0x6EE2, 0x4701, (0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5))
BTH_BATTERY_PID = 2

# System.Devices.Aep.Battery.Level / BatteryLife-style keys seen on Win10/11.
EXTRA_BATTERY_KEYS = (
    ((0xA995CD20, 0x4C4E, 0x4E9B, (0x8A, 0x41, 0x0B, 0x4F, 0x6C, 0x8D, 0x4F, 0x3A)), 2),
    ((0xA8B865DD, 0x2E3D, 0x4094, (0xAD, 0x97, 0xE5, 0x93, 0xA7, 0x0C, 0x75, 0xD6)), 4),
)

DEVPKEY_NAME = ((0xB725F130, 0x47EF, 0x101A, (0xA5, 0xF1, 0x02, 0x60, 0x8C, 0x9E, 0xEB, 0xAC)), 10)
DEVPKEY_FRIENDLY = ((0xA45C254E, 0xDF1C, 0x4EFD, (0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0)), 14)
DEVPKEY_DEVCLASS = ((0xA45C254E, 0xDF1C, 0x4EFD, (0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0)), 8)
DEVPKEY_IS_CONNECTED = ((0x78C34FC8, 0x104A, 0x4ACA, (0x9E, 0xA4, 0x52, 0x4D, 0x52, 0x93, 0x84, 0x80)), 55)
DEVPKEY_CONTAINER_IS_CONNECTED = (
    (0x78C34FC8, 0x104A, 0x4ACA, (0x9E, 0xA4, 0x52, 0x4D, 0x52, 0x93, 0x84, 0x80)),
    51,
)

GUID_DEVCLASS_BLUETOOTH = (0xE0CBF06C, 0xCD8B, 0x4647, (0xBB, 0x8A, 0x26, 0x3B, 0x43, 0xF0, 0xF9, 0x74))
GUID_BLUETOOTHLE_DEVICE_INTERFACE = (
    0x781AEE19,
    0x7733,
    0x4CE4,
    (0xBD, 0xD0, 0x7F, 0x11, 0xCE, 0x3D, 0xBB, 0x24),
)

DIGCF_PRESENT = 0x00000002
DIGCF_ALLCLASSES = 0x00000004
DIGCF_DEVICEINTERFACE = 0x00000010

DEVPROP_TYPE_UINT32 = 0x00000007
DEVPROP_TYPE_BYTE = 0x00000003
DEVPROP_TYPE_BOOLEAN = 0x00000011
DEVPROP_TYPE_STRING = 0x00000012
DEVPROP_TYPE_MASK = 0x00000FFF

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
FILE_SHARE_READ = 0x00000001
FILE_SHARE_WRITE = 0x00000002
OPEN_EXISTING = 3
FILE_ATTRIBUTE_NORMAL = 0x00000080
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

BLUETOOTH_GATT_FLAG_NONE = 0x00000000
BLUETOOTH_GATT_FLAG_FORCE_READ_FROM_DEVICE = 0x00000004

GATT_CHARACTERISTIC_UUID_BATTERY_LEVEL = 0x2A19
GATT_SERVICE_UUID_BATTERY = 0x180F


class GUID(ctypes.Structure):
    _fields_ = (
        ("Data1", wintypes.DWORD),
        ("Data2", wintypes.WORD),
        ("Data3", wintypes.WORD),
        ("Data4", wintypes.BYTE * 8),
    )


class DEVPROPKEY(ctypes.Structure):
    _fields_ = (("fmtid", GUID), ("pid", wintypes.ULONG))


class SP_DEVINFO_DATA(ctypes.Structure):
    _fields_ = (
        ("cbSize", wintypes.DWORD),
        ("ClassGuid", GUID),
        ("DevInst", wintypes.DWORD),
        ("Reserved", ctypes.c_void_p),
    )


class SP_DEVICE_INTERFACE_DATA(ctypes.Structure):
    _fields_ = (
        ("cbSize", wintypes.DWORD),
        ("InterfaceClassGuid", GUID),
        ("Flags", wintypes.DWORD),
        ("Reserved", ctypes.c_void_p),
    )


class BTH_LE_UUID(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = (("ShortUuid", wintypes.USHORT), ("LongUuid", GUID))

    _anonymous_ = ("u",)
    _fields_ = (("IsShortUuid", wintypes.BOOLEAN), ("u", _U))


class BTH_LE_GATT_SERVICE(ctypes.Structure):
    _fields_ = (
        ("ServiceUuid", BTH_LE_UUID),
        ("AttributeHandle", wintypes.USHORT),
    )


class BTH_LE_GATT_CHARACTERISTIC(ctypes.Structure):
    _fields_ = (
        ("ServiceHandle", wintypes.USHORT),
        ("CharacteristicUuid", BTH_LE_UUID),
        ("AttributeHandle", wintypes.USHORT),
        ("CharacteristicValueHandle", wintypes.USHORT),
        ("IsBroadcastable", wintypes.BOOLEAN),
        ("IsReadable", wintypes.BOOLEAN),
        ("IsWritable", wintypes.BOOLEAN),
        ("IsWritableWithoutResponse", wintypes.BOOLEAN),
        ("IsSignedWritable", wintypes.BOOLEAN),
        ("IsNotifiable", wintypes.BOOLEAN),
        ("IsIndicatable", wintypes.BOOLEAN),
        ("HasExtendedProperties", wintypes.BOOLEAN),
    )


class BTH_LE_GATT_CHARACTERISTIC_VALUE(ctypes.Structure):
    _fields_ = (("DataSize", wintypes.ULONG), ("Data", wintypes.BYTE * 1))


def _guid(parts: tuple) -> GUID:
    # Mask to 32-bit unsigned — raw 0xE0CBF06C overflows signed c_long on some builds.
    data1 = int(parts[0]) & 0xFFFFFFFF
    data2 = int(parts[1]) & 0xFFFF
    data3 = int(parts[2]) & 0xFFFF
    data4 = (wintypes.BYTE * 8)(*(int(b) & 0xFF for b in parts[3]))
    return GUID(data1, data2, data3, data4)


def _propkey(parts: tuple, pid: int) -> DEVPROPKEY:
    return DEVPROPKEY(_guid(parts), pid)


def _setupapi():
    api = ctypes.WinDLL("setupapi", use_last_error=True)
    api.SetupDiGetClassDevsW.restype = wintypes.HANDLE
    api.SetupDiGetClassDevsW.argtypes = [
        ctypes.POINTER(GUID),
        wintypes.LPCWSTR,
        wintypes.HWND,
        wintypes.DWORD,
    ]
    api.SetupDiEnumDeviceInfo.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(SP_DEVINFO_DATA)]
    api.SetupDiEnumDeviceInfo.restype = wintypes.BOOL
    api.SetupDiGetDevicePropertyW.restype = wintypes.BOOL
    api.SetupDiGetDevicePropertyW.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(SP_DEVINFO_DATA),
        ctypes.POINTER(DEVPROPKEY),
        ctypes.POINTER(wintypes.ULONG),
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        wintypes.DWORD,
    ]
    api.SetupDiDestroyDeviceInfoList.argtypes = [wintypes.HANDLE]
    api.SetupDiDestroyDeviceInfoList.restype = wintypes.BOOL
    api.SetupDiEnumDeviceInterfaces.restype = wintypes.BOOL
    api.SetupDiEnumDeviceInterfaces.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(SP_DEVINFO_DATA),
        ctypes.POINTER(GUID),
        wintypes.DWORD,
        ctypes.POINTER(SP_DEVICE_INTERFACE_DATA),
    ]
    api.SetupDiGetDeviceInterfaceDetailW.restype = wintypes.BOOL
    api.SetupDiGetDeviceInterfaceDetailW.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(SP_DEVICE_INTERFACE_DATA),
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        ctypes.POINTER(SP_DEVINFO_DATA),
    ]
    return api


def _kernel32():
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    k32.CreateFileW.restype = wintypes.HANDLE
    k32.CreateFileW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    k32.CloseHandle.argtypes = [wintypes.HANDLE]
    k32.CloseHandle.restype = wintypes.BOOL
    return k32


def _read_prop(api, devs, info, key: DEVPROPKEY) -> Any:
    ptype = wintypes.ULONG()
    needed = wintypes.DWORD()
    api.SetupDiGetDevicePropertyW(
        devs,
        ctypes.byref(info),
        ctypes.byref(key),
        ctypes.byref(ptype),
        None,
        0,
        ctypes.byref(needed),
        0,
    )
    size = needed.value
    if size == 0:
        return None
    buf = (ctypes.c_ubyte * size)()
    ok = api.SetupDiGetDevicePropertyW(
        devs,
        ctypes.byref(info),
        ctypes.byref(key),
        ctypes.byref(ptype),
        ctypes.cast(buf, ctypes.POINTER(ctypes.c_ubyte)),
        size,
        ctypes.byref(needed),
        0,
    )
    if not ok:
        return None
    kind = ptype.value & DEVPROP_TYPE_MASK
    raw = bytes(buf)
    if kind == DEVPROP_TYPE_STRING:
        return ctypes.wstring_at(ctypes.addressof(buf))
    if kind == DEVPROP_TYPE_BOOLEAN:
        return bool(raw[0]) if raw else None
    if kind in (DEVPROP_TYPE_BYTE, DEVPROP_TYPE_UINT32):
        return int.from_bytes(raw[:4].ljust(4, b"\x00"), "little")
    if len(raw) == 1:
        return raw[0]
    if len(raw) >= 4:
        return int.from_bytes(raw[:4], "little")
    return None


def _name_matches(name: str) -> bool:
    lowered = name.lower()
    return any(hint in lowered for hint in NAME_HINTS)


@dataclass
class BtDevice:
    name: str
    connected: bool | None = None
    battery: int | None = None
    instance: str = ""
    extras: dict[str, Any] = field(default_factory=dict)


def list_bluetooth_pnp() -> list[BtDevice]:
    api = _setupapi()
    guid = _guid(GUID_DEVCLASS_BLUETOOTH)
    devs = api.SetupDiGetClassDevsW(ctypes.byref(guid), None, None, DIGCF_PRESENT)
    if devs in (None, INVALID_HANDLE_VALUE, 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF):
        # Fall back to all present devices and filter by class name.
        devs = api.SetupDiGetClassDevsW(None, None, None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
        if devs in (None, INVALID_HANDLE_VALUE, 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF):
            return []

    results: list[BtDevice] = []
    try:
        index = 0
        while True:
            info = SP_DEVINFO_DATA()
            info.cbSize = ctypes.sizeof(SP_DEVINFO_DATA)
            if not api.SetupDiEnumDeviceInfo(devs, index, ctypes.byref(info)):
                break
            index += 1
            name = _read_prop(api, devs, info, _propkey(*DEVPKEY_FRIENDLY)) or _read_prop(
                api, devs, info, _propkey(*DEVPKEY_NAME)
            )
            if not isinstance(name, str) or not name:
                continue
            cls_name = _read_prop(api, devs, info, _propkey(*DEVPKEY_DEVCLASS))
            looks_bt = isinstance(cls_name, str) and "bluetooth" in cls_name.lower()
            if not _name_matches(name) and not looks_bt:
                continue
            if not _name_matches(name) and "razer" not in name.lower() and "blackshark" not in name.lower():
                continue

            battery = _read_prop(api, devs, info, _propkey(BTH_BATTERY_FMTID, BTH_BATTERY_PID))
            if battery is None:
                for fmt, pid in EXTRA_BATTERY_KEYS:
                    battery = _read_prop(api, devs, info, _propkey(fmt, pid))
                    if battery is not None:
                        break
            connected = _read_prop(api, devs, info, _propkey(*DEVPKEY_IS_CONNECTED))
            if connected is None:
                connected = _read_prop(api, devs, info, _propkey(*DEVPKEY_CONTAINER_IS_CONNECTED))
            results.append(
                BtDevice(
                    name=name,
                    connected=bool(connected) if connected is not None else None,
                    battery=int(battery) if isinstance(battery, int) else None,
                )
            )
    finally:
        api.SetupDiDestroyDeviceInfoList(devs)
    return results


def _enum_le_paths() -> list[tuple[str, str]]:
    api = _setupapi()
    guid = _guid(GUID_BLUETOOTHLE_DEVICE_INTERFACE)
    flags = DIGCF_PRESENT | DIGCF_DEVICEINTERFACE
    devs = api.SetupDiGetClassDevsW(ctypes.byref(guid), None, None, flags)
    if devs in (None, INVALID_HANDLE_VALUE, 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF):
        return []

    class SP_DEVICE_INTERFACE_DETAIL_DATA_W(ctypes.Structure):
        _fields_ = (("cbSize", wintypes.DWORD), ("DevicePath", wintypes.WCHAR * 1))

    out: list[tuple[str, str]] = []
    try:
        index = 0
        while True:
            iface = SP_DEVICE_INTERFACE_DATA()
            iface.cbSize = ctypes.sizeof(SP_DEVICE_INTERFACE_DATA)
            if not api.SetupDiEnumDeviceInterfaces(devs, None, ctypes.byref(guid), index, ctypes.byref(iface)):
                break
            index += 1
            needed = wintypes.DWORD()
            api.SetupDiGetDeviceInterfaceDetailW(
                devs, ctypes.byref(iface), None, 0, ctypes.byref(needed), None
            )
            size = needed.value
            if size == 0:
                continue
            detail_buf = ctypes.create_string_buffer(size)
            detail = ctypes.cast(detail_buf, ctypes.POINTER(SP_DEVICE_INTERFACE_DETAIL_DATA_W))
            # cbSize is 8 on 64-bit Windows (DWORD + pointer alignment).
            detail.contents.cbSize = 8 if ctypes.sizeof(ctypes.c_void_p) == 8 else 6
            info = SP_DEVINFO_DATA()
            info.cbSize = ctypes.sizeof(SP_DEVINFO_DATA)
            if not api.SetupDiGetDeviceInterfaceDetailW(
                devs,
                ctypes.byref(iface),
                detail_buf,
                size,
                ctypes.byref(needed),
                ctypes.byref(info),
            ):
                continue
            path = ctypes.wstring_at(ctypes.addressof(detail.contents.DevicePath))
            name = _read_prop(api, devs, info, _propkey(*DEVPKEY_FRIENDLY)) or _read_prop(
                api, devs, info, _propkey(*DEVPKEY_NAME)
            )
            if isinstance(name, str):
                out.append((path, name))
    finally:
        api.SetupDiDestroyDeviceInfoList(devs)
    return out


def _uuid_is_short(uuid: BTH_LE_UUID, short: int) -> bool:
    if uuid.IsShortUuid:
        return uuid.ShortUuid == short
    # Bluetooth SIG 16-bit UUID in 128-bit form: 0000XXXX-0000-1000-8000-00805f9b34fb
    return uuid.LongUuid.Data1 == short and uuid.LongUuid.Data2 == 0x0000 and uuid.LongUuid.Data3 == 0x1000


def _read_gatt_battery(path: str) -> int | None:
    try:
        gatt = ctypes.WinDLL("BluetoothAPIs")
    except OSError:
        return None
    k32 = _kernel32()
    handle = k32.CreateFileW(
        path,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        None,
    )
    if handle in (None, INVALID_HANDLE_VALUE, 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF):
        handle = k32.CreateFileW(
            path,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    if handle in (None, INVALID_HANDLE_VALUE, 0xFFFFFFFF, 0xFFFFFFFFFFFFFFFF):
        return None

    try:
        svc_count = wintypes.USHORT()
        hr = gatt.BluetoothGATTGetServices(handle, 0, None, ctypes.byref(svc_count), BLUETOOTH_GATT_FLAG_NONE)
        # ERROR_MORE_DATA = 234, HRESULT-style 0x8007007A also appears.
        if svc_count.value == 0:
            return None
        services = (BTH_LE_GATT_SERVICE * svc_count.value)()
        hr = gatt.BluetoothGATTGetServices(
            handle, svc_count.value, services, ctypes.byref(svc_count), BLUETOOTH_GATT_FLAG_NONE
        )
        if hr not in (0,):
            # Some stacks return the count as success with S_OK anyway.
            pass
        battery_svc = None
        for svc in services[: svc_count.value]:
            if _uuid_is_short(svc.ServiceUuid, GATT_SERVICE_UUID_BATTERY):
                battery_svc = svc
                break
        if battery_svc is None:
            return None

        char_count = wintypes.USHORT()
        gatt.BluetoothGATTGetCharacteristics(
            handle, ctypes.byref(battery_svc), 0, None, ctypes.byref(char_count), BLUETOOTH_GATT_FLAG_NONE
        )
        if char_count.value == 0:
            return None
        chars = (BTH_LE_GATT_CHARACTERISTIC * char_count.value)()
        gatt.BluetoothGATTGetCharacteristics(
            handle,
            ctypes.byref(battery_svc),
            char_count.value,
            chars,
            ctypes.byref(char_count),
            BLUETOOTH_GATT_FLAG_NONE,
        )
        level_char = None
        for ch in chars[: char_count.value]:
            if _uuid_is_short(ch.CharacteristicUuid, GATT_CHARACTERISTIC_UUID_BATTERY_LEVEL):
                level_char = ch
                break
        if level_char is None:
            return None

        needed = wintypes.USHORT()
        gatt.BluetoothGATTGetCharacteristicValue(
            handle,
            ctypes.byref(level_char),
            0,
            None,
            ctypes.byref(needed),
            BLUETOOTH_GATT_FLAG_FORCE_READ_FROM_DEVICE,
        )
        size = max(int(needed.value), ctypes.sizeof(BTH_LE_GATT_CHARACTERISTIC_VALUE))
        buf = ctypes.create_string_buffer(size)
        value = ctypes.cast(buf, ctypes.POINTER(BTH_LE_GATT_CHARACTERISTIC_VALUE))
        hr = gatt.BluetoothGATTGetCharacteristicValue(
            handle,
            ctypes.byref(level_char),
            size,
            value,
            ctypes.byref(needed),
            BLUETOOTH_GATT_FLAG_FORCE_READ_FROM_DEVICE,
        )
        if value.contents.DataSize < 1:
            hr = gatt.BluetoothGATTGetCharacteristicValue(
                handle,
                ctypes.byref(level_char),
                size,
                value,
                ctypes.byref(needed),
                BLUETOOTH_GATT_FLAG_NONE,
            )
        if value.contents.DataSize < 1:
            return None
        return int(value.contents.Data[0])
    except Exception:
        return None
    finally:
        k32.CloseHandle(handle)


def read_bluetooth_battery() -> dict[str, Any] | None:
    """Prefer a connected BlackShark-named device with a battery reading."""
    pnp = list_bluetooth_pnp()
    matching = [d for d in pnp if _name_matches(d.name) or "blackshark" in d.name.lower()]
    matching.sort(key=lambda d: (d.battery is not None, bool(d.connected)), reverse=True)

    for device in matching:
        if device.battery is None:
            continue
        percent = max(0, min(100, int(device.battery)))
        return {
            "ok": True,
            "source": "bluetooth-pnp",
            "transport": "Bluetooth",
            "product": device.name,
            "percent": percent,
            "charging": False,
            "connected": device.connected,
        }

    try:
        le_paths = _enum_le_paths()
    except Exception:
        le_paths = []
    for path, name in le_paths:
        if name and not (_name_matches(name) or "blackshark" in name.lower() or "razer" in name.lower()):
            continue
        if name and "razer" not in name.lower() and "blackshark" not in name.lower():
            continue
        percent = _read_gatt_battery(path)
        if percent is None:
            continue
        percent = max(0, min(100, int(percent)))
        return {
            "ok": True,
            "source": "bluetooth-gatt",
            "transport": "Bluetooth",
            "product": name or "Razer BlackShark V2 HS BT",
            "percent": percent,
            "charging": False,
        }

    if matching:
        best = matching[0]
        return {
            "ok": False,
            "source": "bluetooth-pnp",
            "transport": "Bluetooth",
            "product": best.name,
            "error": "Windows bu cihaz için pil yüzdesi yayınlamıyor (HFP/GATT yok).",
            "connected": best.connected,
        }
    return None
