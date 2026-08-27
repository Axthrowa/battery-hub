"""HID probe that runs under Smart App Control.

SAC blocks unsigned executables, which includes anything hidapi-based that has
just been compiled — so the usual way of poking at a device during development
is unavailable. This script loads no native code of its own: it drives Windows'
own signed hid.dll / setupapi.dll through ctypes, under a python.exe that is
already trusted, and is therefore allowed to run as-is.

Usage:
    python scripts/hid-probe.py            # query the Aula battery frame
    python scripts/hid-probe.py --list     # every HID collection on the machine
"""
import ctypes as C
import sys
from ctypes import wintypes as W

setupapi = C.WinDLL("setupapi", use_last_error=True)
hid = C.WinDLL("hid", use_last_error=True)
k32 = C.WinDLL("kernel32", use_last_error=True)

# ctypes defaults every return value to a 32-bit int, which silently truncates
# the 64-bit HDEVINFO and HANDLE values these APIs hand back.
setupapi.SetupDiGetClassDevsW.restype = C.c_void_p
setupapi.SetupDiGetClassDevsW.argtypes = [C.c_void_p, C.c_wchar_p, C.c_void_p, W.DWORD]
setupapi.SetupDiEnumDeviceInterfaces.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p,
                                                 W.DWORD, C.c_void_p]
setupapi.SetupDiGetDeviceInterfaceDetailW.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p,
                                                      W.DWORD, C.c_void_p, C.c_void_p]
setupapi.SetupDiDestroyDeviceInfoList.argtypes = [C.c_void_p]
k32.CreateFileW.restype = C.c_void_p
k32.CreateFileW.argtypes = [C.c_wchar_p, W.DWORD, W.DWORD, C.c_void_p, W.DWORD,
                            W.DWORD, C.c_void_p]
k32.CreateEventW.restype = C.c_void_p
k32.CreateEventW.argtypes = [C.c_void_p, W.BOOL, W.BOOL, C.c_wchar_p]
k32.CloseHandle.argtypes = [C.c_void_p]
k32.ReadFile.argtypes = [C.c_void_p, C.c_void_p, W.DWORD, C.c_void_p, C.c_void_p]
k32.WaitForSingleObject.argtypes = [C.c_void_p, W.DWORD]
k32.CancelIo.argtypes = [C.c_void_p]
k32.GetOverlappedResult.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, W.BOOL]
for fn in (hid.HidD_GetAttributes, hid.HidD_GetPreparsedData, hid.HidD_GetProductString,
           hid.HidD_SetOutputReport, hid.HidD_GetFeature, hid.HidD_SetFeature):
    fn.argtypes = None
hid.HidD_GetAttributes.argtypes = [C.c_void_p, C.c_void_p]
hid.HidD_GetPreparsedData.argtypes = [C.c_void_p, C.c_void_p]
hid.HidD_FreePreparsedData.argtypes = [C.c_void_p]
hid.HidP_GetCaps.argtypes = [C.c_void_p, C.c_void_p]
hid.HidD_GetProductString.argtypes = [C.c_void_p, C.c_void_p, W.ULONG]
hid.HidD_SetOutputReport.argtypes = [C.c_void_p, C.c_void_p, W.ULONG]

GENERIC_READ, GENERIC_WRITE = 0x80000000, 0x40000000
FILE_SHARE_READ, FILE_SHARE_WRITE = 1, 2
OPEN_EXISTING, FILE_FLAG_OVERLAPPED = 3, 0x40000000
INVALID_HANDLE = C.c_void_p(-1).value
DIGCF_PRESENT, DIGCF_DEVICEINTERFACE = 0x02, 0x10


class GUID(C.Structure):
    _fields_ = [("d1", W.DWORD), ("d2", W.WORD), ("d3", W.WORD), ("d4", C.c_ubyte * 8)]


class SP_DEVICE_INTERFACE_DATA(C.Structure):
    _fields_ = [("cbSize", W.DWORD), ("InterfaceClassGuid", GUID),
                ("Flags", W.DWORD), ("Reserved", C.POINTER(W.ULONG))]


class HIDD_ATTRIBUTES(C.Structure):
    _fields_ = [("Size", W.ULONG), ("VendorID", W.USHORT),
                ("ProductID", W.USHORT), ("VersionNumber", W.USHORT)]


class HIDP_CAPS(C.Structure):
    _fields_ = [("Usage", W.USHORT), ("UsagePage", W.USHORT),
                ("InputReportByteLength", W.USHORT), ("OutputReportByteLength", W.USHORT),
                ("FeatureReportByteLength", W.USHORT), ("Reserved", W.USHORT * 17),
                ("NumberLinkCollectionNodes", W.USHORT), ("NumberInputButtonCaps", W.USHORT),
                ("NumberInputValueCaps", W.USHORT), ("NumberInputDataIndices", W.USHORT),
                ("NumberOutputButtonCaps", W.USHORT), ("NumberOutputValueCaps", W.USHORT),
                ("NumberOutputDataIndices", W.USHORT), ("NumberFeatureButtonCaps", W.USHORT),
                ("NumberFeatureValueCaps", W.USHORT), ("NumberFeatureDataIndices", W.USHORT)]


class OVERLAPPED(C.Structure):
    _fields_ = [("Internal", C.POINTER(W.ULONG)), ("InternalHigh", C.POINTER(W.ULONG)),
                ("Offset", W.DWORD), ("OffsetHigh", W.DWORD), ("hEvent", W.HANDLE)]


def interface_paths():
    guid = GUID()
    hid.HidD_GetHidGuid(C.byref(guid))
    info = setupapi.SetupDiGetClassDevsW(C.byref(guid), None, None,
                                         DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
    if info == INVALID_HANDLE:
        raise OSError("SetupDiGetClassDevs failed")
    index, out = 0, []
    while True:
        iface = SP_DEVICE_INTERFACE_DATA()
        iface.cbSize = C.sizeof(SP_DEVICE_INTERFACE_DATA)
        if not setupapi.SetupDiEnumDeviceInterfaces(info, None, C.byref(guid), index,
                                                    C.byref(iface)):
            break
        index += 1
        need = W.DWORD()
        setupapi.SetupDiGetDeviceInterfaceDetailW(info, C.byref(iface), None, 0,
                                                  C.byref(need), None)
        buf = C.create_string_buffer(need.value)
        C.cast(buf, C.POINTER(W.DWORD))[0] = 8  # cbSize for the 64-bit detail struct
        if setupapi.SetupDiGetDeviceInterfaceDetailW(info, C.byref(iface), buf, need,
                                                     C.byref(need), None):
            out.append(C.wstring_at(C.addressof(buf) + C.sizeof(W.DWORD)))
    setupapi.SetupDiDestroyDeviceInfoList(info)
    return out


def open_device(path, overlapped=True):
    flags = FILE_FLAG_OVERLAPPED if overlapped else 0
    handle = k32.CreateFileW(path, GENERIC_READ | GENERIC_WRITE,
                             FILE_SHARE_READ | FILE_SHARE_WRITE, None,
                             OPEN_EXISTING, flags, None)
    return None if handle == INVALID_HANDLE else handle


def describe(handle):
    attrs = HIDD_ATTRIBUTES()
    attrs.Size = C.sizeof(attrs)
    if not hid.HidD_GetAttributes(handle, C.byref(attrs)):
        return None
    pre = C.c_void_p()
    if not hid.HidD_GetPreparsedData(handle, C.byref(pre)):
        return None
    caps = HIDP_CAPS()
    hid.HidP_GetCaps(pre, C.byref(caps))
    hid.HidD_FreePreparsedData(pre)
    name = C.create_unicode_buffer(128)
    hid.HidD_GetProductString(handle, name, C.sizeof(name))
    return {"vid": attrs.VendorID, "pid": attrs.ProductID, "usage_page": caps.UsagePage,
            "usage": caps.Usage, "in_len": caps.InputReportByteLength,
            "out_len": caps.OutputReportByteLength,
            "feat_len": caps.FeatureReportByteLength, "product": name.value}


def read_input(handle, length, timeout_ms):
    event = k32.CreateEventW(None, True, False, None)
    ov = OVERLAPPED()
    ov.hEvent = event
    buf = C.create_string_buffer(length)
    got = W.DWORD()
    ok = k32.ReadFile(handle, buf, length, C.byref(got), C.byref(ov))
    if not ok:
        if C.get_last_error() != 997:  # ERROR_IO_PENDING
            k32.CloseHandle(event)
            return None
        if k32.WaitForSingleObject(event, timeout_ms) != 0:
            k32.CancelIo(handle)
            k32.CloseHandle(event)
            return None
        k32.GetOverlappedResult(handle, C.byref(ov), C.byref(got), False)
    k32.CloseHandle(event)
    return buf.raw[: got.value]


def crc(report_id, payload):
    return (report_id + sum(payload[:-1])) % 256


def hexs(b):
    return " ".join(f"{x:02X}" for x in b)


def list_all():
    for path in interface_paths():
        handle = open_device(path, overlapped=False)
        if handle is None:
            continue
        info = describe(handle)
        if info:
            print(f"{info['vid']:04X}:{info['pid']:04X} "
                  f"usage_page=0x{info['usage_page']:04X} usage=0x{info['usage']:04X} "
                  f"in={info['in_len']:>3} out={info['out_len']:>3} feat={info['feat_len']:>3}  "
                  f"{info['product']}")
        k32.CloseHandle(handle)
    return 0


def main():
    if "--list" in sys.argv:
        return list_all()
    targets = []
    for path in interface_paths():
        handle = open_device(path)
        if handle is None:
            continue
        info = describe(handle)
        if info and info["vid"] == 0x3554 and info["pid"] == 0xFA09 and info["usage_page"] == 0xFF02:
            targets.append((path, handle, info))
        else:
            k32.CloseHandle(handle)

    if not targets:
        print("Aula FF02 collection not found")
        return 1

    path, handle, info = targets[0]
    print(f"device: VID 0x{info['vid']:04X} PID 0x{info['pid']:04X} "
          f"usage_page 0x{info['usage_page']:04X} in={info['in_len']} out={info['out_len']}")
    print(f"product: {info['product']}")

    for label, cmd, marker in (("battery", [0x4A, 0, 0, 0], 0x4A),
                               ("uuid", [0x05, 0x01, 0, 0, 0, 0, 0], 0x05)):
        payload = bytearray(19)
        payload[: len(cmd)] = bytes(cmd)
        payload[18] = crc(0x13, payload)
        frame = bytes([0x13]) + bytes(payload)
        out = C.create_string_buffer(frame, info["out_len"])
        # The 2.4 GHz link stays quiet through the first frame now and then, so
        # ask again rather than reporting a keyboard that is switched on.
        reply = None
        for _ in range(5):
            if not hid.HidD_SetOutputReport(handle, out, info["out_len"]):
                print(f"{label}: SetOutputReport failed ({C.get_last_error()})")
                break
            reply = read_input(handle, info["in_len"], 400)
            if reply:
                break
        if not reply:
            print(f"{label}: no reply")
            continue
        body = reply[1:] if reply[0] == 0x13 else reply
        print(f"{label}: RX {hexs(reply[:20])}")
        if body and body[0] == marker:
            n = body[3]
            data = body[4 : 4 + n]
            if label == "battery":
                print(f"  -> {data[0]}%  status=0x{data[1]:02X}")
            else:
                uid = int.from_bytes(data[:6], "big")
                print(f"  -> uuid {uid} (0x{uid:012X})")
    k32.CloseHandle(handle)
    return 0


if __name__ == "__main__":
    sys.exit(main())
