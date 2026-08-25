// BlackSharkBattery — pure Win32 tray battery monitor (GDI+ UI, async HID poll)
// Hardware HID/Bluetooth logic preserved. Build: build.bat

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <objidl.h>
#include <shellapi.h>
#include <setupapi.h>
#include <hidsdi.h>
#include <devpropdef.h>
#include <commctrl.h>
#include <strsafe.h>
#include <gdiplus.h>
#include <atomic>
#include <thread>
#include <mutex>
#include <cstring>
#include <cwchar>
#include <cstdint>
#include "resource.h"

#pragma comment(lib, "setupapi.lib")
#pragma comment(lib, "hid.lib")
#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "gdi32.lib")
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "advapi32.lib")
#pragma comment(lib, "comctl32.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "gdiplus.lib")

using namespace Gdiplus;

namespace {

constexpr wchar_t kWndClass[] = L"BlackSharkBatteryWnd";
constexpr wchar_t kAppTitle[] = L"BlackShark Battery";
constexpr wchar_t kRegApp[] = L"Software\\BlackSharkBattery";
constexpr wchar_t kRegRun[] = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
constexpr wchar_t kRunValue[] = L"BlackSharkBattery";

constexpr UINT WM_TRAY = WM_APP + 1;
constexpr UINT WM_BATTERY_UPDATED = WM_APP + 2;
constexpr UINT_PTR TIMER_POLL = 1;

constexpr UINT ID_HEADER = 1000;
constexpr UINT ID_REFRESH = 1002;
constexpr UINT ID_STARTUP = 1100;
constexpr UINT ID_EXIT = 1001;
constexpr UINT ID_SEP = 1003;

constexpr USHORT kVid = 0x1532;
constexpr USHORT kPidDongle = 0x0565;
constexpr USHORT kPidWired = 0x056E;
constexpr BYTE kReportId = 0x02;
constexpr BYTE kRfWake = 0x05;
constexpr BYTE kChannel = 0x60;
constexpr int kReportLen = 64;
constexpr int kCrcIndex = 62;
constexpr BYTE kClassHeadset = 0x80;
constexpr BYTE kCmdBattery = 0x21;
constexpr BYTE kCmdCharging = 0x2A;
constexpr BYTE kCmdLink = 0x20;

struct Settings {
    BOOL runAtStartup = FALSE;
    BOOL notifyLow = TRUE;
    int pollSeconds = 60;
    int lowThreshold = 15;
};

struct BatteryState {
    BOOL ok = FALSE;
    int percent = -1;
    BOOL charging = FALSE;
    wchar_t transport[32]{};
    wchar_t product[96]{};
    wchar_t error[160]{};
};

HWND g_hwnd = nullptr;
HINSTANCE g_hInst = nullptr;
NOTIFYICONDATAW g_nid{};
HICON g_icon = nullptr;
HICON g_appIcon = nullptr;
Settings g_settings{};
BatteryState g_battery{};
BOOL g_notifiedLow = FALSE;
UINT g_taskbarCreated = 0;
ULONG_PTR g_gdiplusToken = 0;

std::atomic<int> g_percent{-1};
std::atomic<int> g_charging{0};
std::atomic<int> g_ok{0};
std::atomic<int> g_pollSeconds{60};
std::atomic<int> g_stopWorker{0};
std::mutex g_stateMutex;
HANDLE g_wakeEvent = nullptr;
std::thread g_worker;

constexpr GUID kTrayGuid = {0x8F3C2A91, 0x6B4E, 0x4D1A, {0x9C, 0x7F, 0x2E, 0x5A, 0x1B, 0x0D, 0x4C, 0x33}};

int Clamp(int v, int lo, int hi) {
    if (v < lo) return lo;
    if (v > hi) return hi;
    return v;
}

void LoadSettings() {
    g_settings = Settings{};
    HKEY key = nullptr;
    if (RegOpenKeyExW(HKEY_CURRENT_USER, kRegApp, 0, KEY_READ, &key) == ERROR_SUCCESS) {
        DWORD type = 0, data = 0, size = sizeof(data);
        if (RegQueryValueExW(key, L"NotifyLow", nullptr, &type, reinterpret_cast<LPBYTE>(&data), &size) == ERROR_SUCCESS && type == REG_DWORD)
            g_settings.notifyLow = data ? TRUE : FALSE;
        size = sizeof(data);
        if (RegQueryValueExW(key, L"PollSeconds", nullptr, &type, reinterpret_cast<LPBYTE>(&data), &size) == ERROR_SUCCESS && type == REG_DWORD)
            g_settings.pollSeconds = Clamp(static_cast<int>(data), 15, 300);
        size = sizeof(data);
        if (RegQueryValueExW(key, L"LowThreshold", nullptr, &type, reinterpret_cast<LPBYTE>(&data), &size) == ERROR_SUCCESS && type == REG_DWORD)
            g_settings.lowThreshold = Clamp(static_cast<int>(data), 5, 50);
        RegCloseKey(key);
    }
    HKEY run = nullptr;
    g_settings.runAtStartup = FALSE;
    if (RegOpenKeyExW(HKEY_CURRENT_USER, kRegRun, 0, KEY_READ, &run) == ERROR_SUCCESS) {
        wchar_t buf[MAX_PATH]{};
        DWORD cb = sizeof(buf);
        DWORD type = 0;
        if (RegQueryValueExW(run, kRunValue, nullptr, &type, reinterpret_cast<LPBYTE>(buf), &cb) == ERROR_SUCCESS)
            g_settings.runAtStartup = TRUE;
        RegCloseKey(run);
    }
    g_pollSeconds.store(g_settings.pollSeconds);
}

void SaveSettings() {
    HKEY key = nullptr;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, kRegApp, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr) == ERROR_SUCCESS) {
        DWORD v = g_settings.notifyLow ? 1 : 0;
        RegSetValueExW(key, L"NotifyLow", 0, REG_DWORD, reinterpret_cast<const BYTE*>(&v), sizeof(v));
        v = static_cast<DWORD>(g_settings.pollSeconds);
        RegSetValueExW(key, L"PollSeconds", 0, REG_DWORD, reinterpret_cast<const BYTE*>(&v), sizeof(v));
        v = static_cast<DWORD>(g_settings.lowThreshold);
        RegSetValueExW(key, L"LowThreshold", 0, REG_DWORD, reinterpret_cast<const BYTE*>(&v), sizeof(v));
        RegCloseKey(key);
    }
    g_pollSeconds.store(g_settings.pollSeconds);
}

void SetStartup(BOOL enable) {
    HKEY run = nullptr;
    if (RegOpenKeyExW(HKEY_CURRENT_USER, kRegRun, 0, KEY_SET_VALUE, &run) != ERROR_SUCCESS)
        return;
    if (enable) {
        wchar_t path[MAX_PATH]{};
        GetModuleFileNameW(nullptr, path, MAX_PATH);
        wchar_t quoted[MAX_PATH + 3]{};
        StringCchPrintfW(quoted, ARRAYSIZE(quoted), L"\"%s\"", path);
        RegSetValueExW(run, kRunValue, 0, REG_SZ, reinterpret_cast<const BYTE*>(quoted),
                       static_cast<DWORD>((wcslen(quoted) + 1) * sizeof(wchar_t)));
    } else {
        RegDeleteValueW(run, kRunValue);
    }
    RegCloseKey(run);
    g_settings.runAtStartup = enable;
}

// ----- Hardware protocol (unchanged) -----

BYTE XorChecksum(const BYTE* buf) {
    BYTE crc = 0;
    for (int i = 0; i < kCrcIndex; ++i) crc ^= buf[i];
    return crc;
}

void BuildQuery(BYTE* buf, BYTE cmd, BOOL dongle) {
    ZeroMemory(buf, kReportLen);
    buf[0] = kReportId;
    buf[2] = kChannel;
    buf[6] = 0x04;
    buf[10] = cmd;
    buf[12] = 0x00;
    if (dongle) {
        buf[9] = kClassHeadset;
        buf[kCrcIndex] = XorChecksum(buf);
    }
}

BOOL ParseReply(const BYTE* data, int len, BYTE expectedCmd, int* outValue) {
    if (len <= 13 || !outValue) return FALSE;
    const BYTE* p = data;
    if (data[0] == kReportId) {
        p = data;
    } else if (len > 14 && data[1] == kReportId) {
        p = data + 1;
        len -= 1;
    } else if (data[0] != kReportId) {
        return FALSE;
    }
    if (len <= 13) return FALSE;
    if (p[10] != expectedCmd) return FALSE;
    if (p[11] != 0x01) return FALSE;
    *outValue = p[13];
    return TRUE;
}

BOOL PathLooksPreferred(const wchar_t* path) {
    if (!path) return FALSE;
    return wcsstr(path, L"Col04") != nullptr || wcsstr(path, L"col04") != nullptr;
}

BOOL PathIsTarget(const wchar_t* path, USHORT* outPid) {
    if (!path) return FALSE;
    if (!wcsstr(path, L"VID_1532") && !wcsstr(path, L"vid_1532")) return FALSE;
    if (wcsstr(path, L"PID_0565") || wcsstr(path, L"pid_0565")) {
        if (outPid) *outPid = kPidDongle;
        return TRUE;
    }
    if (wcsstr(path, L"PID_056E") || wcsstr(path, L"pid_056e")) {
        if (outPid) *outPid = kPidWired;
        return TRUE;
    }
    return FALSE;
}

void DrainReads(HANDLE h) {
    BYTE buf[65]{};
    DWORD rd = 0;
    for (int i = 0; i < 24; ++i) {
        OVERLAPPED ov{};
        ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!ov.hEvent) break;
        BOOL ok = ReadFile(h, buf, sizeof(buf), &rd, &ov);
        if (!ok && GetLastError() == ERROR_IO_PENDING) {
            if (WaitForSingleObject(ov.hEvent, 5) != WAIT_OBJECT_0) {
                CancelIo(h);
                CloseHandle(ov.hEvent);
                break;
            }
            GetOverlappedResult(h, &ov, &rd, FALSE);
        } else if (!ok) {
            CloseHandle(ov.hEvent);
            break;
        }
        CloseHandle(ov.hEvent);
        if (rd == 0) break;
    }
}

BOOL WriteReport(HANDLE h, const BYTE* data, DWORD len) {
    DWORD wr = 0;
    if (WriteFile(h, data, len, &wr, nullptr) && wr == len) return TRUE;
    return HidD_SetOutputReport(h, (PVOID)data, len) ? TRUE : FALSE;
}

BOOL ReadReportTimed(HANDLE h, BYTE* buf, DWORD cap, DWORD timeoutMs, DWORD* outLen) {
    OVERLAPPED ov{};
    ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!ov.hEvent) return FALSE;
    DWORD rd = 0;
    BOOL ok = ReadFile(h, buf, cap, &rd, &ov);
    if (!ok) {
        if (GetLastError() != ERROR_IO_PENDING) {
            CloseHandle(ov.hEvent);
            return FALSE;
        }
        DWORD wait = WaitForSingleObject(ov.hEvent, timeoutMs);
        if (wait != WAIT_OBJECT_0) {
            CancelIo(h);
            CloseHandle(ov.hEvent);
            return FALSE;
        }
        if (!GetOverlappedResult(h, &ov, &rd, FALSE)) {
            CloseHandle(ov.hEvent);
            return FALSE;
        }
    }
    CloseHandle(ov.hEvent);
    if (outLen) *outLen = rd;
    return rd > 0;
}

BOOL QueryByte(HANDLE h, BYTE cmd, BOOL dongle, int timeoutMs, int* outValue) {
    BYTE report[kReportLen]{};
    BuildQuery(report, cmd, dongle);
    DrainReads(h);
    if (!WriteReport(h, report, kReportLen)) return FALSE;

    const ULONGLONG deadline = GetTickCount64() + static_cast<ULONGLONG>(timeoutMs);
    BYTE buf[80]{};
    while (GetTickCount64() < deadline) {
        DWORD n = 0;
        DWORD slice = static_cast<DWORD>(deadline - GetTickCount64());
        if (slice > 250) slice = 250;
        if (slice < 1) break;
        if (!ReadReportTimed(h, buf, sizeof(buf), slice, &n)) continue;
        int value = 0;
        if (ParseReply(buf, static_cast<int>(n), cmd, &value)) {
            *outValue = value;
            return TRUE;
        }
    }
    return FALSE;
}

BOOL TryDevicePath(const wchar_t* path, USHORT pid, BatteryState* out) {
    HANDLE h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE,
                           FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                           OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
    if (h == INVALID_HANDLE_VALUE) {
        h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                        OPEN_EXISTING, 0, nullptr);
    }
    if (h == INVALID_HANDLE_VALUE) return FALSE;

    const BOOL dongle = (pid == kPidDongle);
    BYTE wake[kReportLen]{};
    wake[0] = kRfWake;
    WriteReport(h, wake, kReportLen);
    Sleep(40);

    int link = 0;
    QueryByte(h, kCmdLink, dongle, 500, &link);

    int percent = 0;
    BOOL ok = QueryByte(h, kCmdBattery, dongle, 1200, &percent);
    if (!ok && dongle) ok = QueryByte(h, kCmdBattery, FALSE, 800, &percent);
    if (!ok) {
        CloseHandle(h);
        return FALSE;
    }

    int charging = 0;
    QueryByte(h, kCmdCharging, dongle, 800, &charging);
    CloseHandle(h);

    out->ok = TRUE;
    out->percent = Clamp(percent, 0, 100);
    out->charging = charging ? TRUE : FALSE;
    StringCchCopyW(out->transport, ARRAYSIZE(out->transport), dongle ? L"2.4 GHz" : L"USB");
    StringCchCopyW(out->product, ARRAYSIZE(out->product), L"Razer BlackShark V2 HyperSpeed");
    out->error[0] = 0;
    return TRUE;
}

BOOL PollHid(BatteryState* out) {
    GUID hidGuid{};
    HidD_GetHidGuid(&hidGuid);
    HDEVINFO info = SetupDiGetClassDevsW(&hidGuid, nullptr, nullptr, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
    if (info == INVALID_HANDLE_VALUE) return FALSE;

    struct Candidate {
        wchar_t path[MAX_PATH * 2];
        USHORT pid;
        int score;
    };
    Candidate list[32]{};
    int count = 0;

    SP_DEVICE_INTERFACE_DATA ifData{};
    ifData.cbSize = sizeof(ifData);
    for (DWORD i = 0; SetupDiEnumDeviceInterfaces(info, nullptr, &hidGuid, i, &ifData); ++i) {
        DWORD needed = 0;
        SetupDiGetDeviceInterfaceDetailW(info, &ifData, nullptr, 0, &needed, nullptr);
        if (needed == 0) continue;
        auto* detail = reinterpret_cast<SP_DEVICE_INTERFACE_DETAIL_DATA_W*>(LocalAlloc(LPTR, needed));
        if (!detail) continue;
        detail->cbSize = sizeof(SP_DEVICE_INTERFACE_DETAIL_DATA_W);
        if (!SetupDiGetDeviceInterfaceDetailW(info, &ifData, detail, needed, nullptr, nullptr)) {
            LocalFree(detail);
            continue;
        }
        USHORT pid = 0;
        if (PathIsTarget(detail->DevicePath, &pid) && count < 32) {
            StringCchCopyW(list[count].path, ARRAYSIZE(list[count].path), detail->DevicePath);
            list[count].pid = pid;
            list[count].score = PathLooksPreferred(detail->DevicePath) ? 100 : 10;
            if (pid == kPidDongle) list[count].score += 1;
            ++count;
        }
        LocalFree(detail);
    }
    SetupDiDestroyDeviceInfoList(info);

    for (int i = 1; i < count; ++i) {
        Candidate key = list[i];
        int j = i - 1;
        while (j >= 0 && list[j].score < key.score) {
            list[j + 1] = list[j];
            --j;
        }
        list[j + 1] = key;
    }

    BOOL sawTarget = count > 0;
    for (int i = 0; i < count; ++i) {
        if (TryDevicePath(list[i].path, list[i].pid, out))
            return TRUE;
    }

    if (sawTarget) {
        out->ok = FALSE;
        StringCchCopyW(out->transport, ARRAYSIZE(out->transport), L"2.4 GHz");
        StringCchCopyW(out->product, ARRAYSIZE(out->product), L"Razer BlackShark V2 HyperSpeed");
        StringCchCopyW(out->error, ARRAYSIZE(out->error),
                       L"Dongle bulundu ama kulaklik yanit vermedi.");
        return TRUE;
    }
    return FALSE;
}

BOOL PollBluetooth(BatteryState* out) {
    DEVPROPKEY batteryKey{};
    batteryKey.fmtid = {0x104EA319, 0x6EE2, 0x4701, {0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5}};
    batteryKey.pid = 2;
    DEVPROPKEY friendlyKey{};
    friendlyKey.fmtid = {0xA45C254E, 0xDF1C, 0x4EFD, {0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0}};
    friendlyKey.pid = 14;

    GUID btClass = {0xE0CBF06C, 0xCD8B, 0x4647, {0xBB, 0x8A, 0x26, 0x3B, 0x43, 0xF0, 0xF9, 0x74}};
    HDEVINFO info = SetupDiGetClassDevsW(&btClass, nullptr, nullptr, DIGCF_PRESENT);
    if (info == INVALID_HANDLE_VALUE) return FALSE;

    SP_DEVINFO_DATA dev{};
    dev.cbSize = sizeof(dev);
    BOOL found = FALSE;
    for (DWORD i = 0; SetupDiEnumDeviceInfo(info, i, &dev); ++i) {
        DEVPROPTYPE ptype = 0;
        wchar_t name[256]{};
        DWORD needed = 0;
        if (!SetupDiGetDevicePropertyW(info, &dev, &friendlyKey, &ptype, reinterpret_cast<PBYTE>(name),
                                       sizeof(name), &needed, 0))
            continue;
        wchar_t lower[256]{};
        StringCchCopyW(lower, ARRAYSIZE(lower), name);
        CharLowerBuffW(lower, static_cast<DWORD>(wcslen(lower)));
        if (!wcsstr(lower, L"blackshark") && !wcsstr(lower, L"razer")) continue;

        BYTE raw[16]{};
        needed = 0;
        ptype = 0;
        if (!SetupDiGetDevicePropertyW(info, &dev, &batteryKey, &ptype, raw, sizeof(raw), &needed, 0))
            continue;
        int percent = (needed >= 4) ? *reinterpret_cast<int*>(raw) : raw[0];
        out->ok = TRUE;
        out->percent = Clamp(percent, 0, 100);
        out->charging = FALSE;
        StringCchCopyW(out->transport, ARRAYSIZE(out->transport), L"Bluetooth");
        StringCchCopyW(out->product, ARRAYSIZE(out->product), name);
        out->error[0] = 0;
        found = TRUE;
        break;
    }
    SetupDiDestroyDeviceInfoList(info);
    return found;
}

void PollBattery() {
    BatteryState st{};
    if (!PollHid(&st)) {
        if (!PollBluetooth(&st)) {
            st.ok = FALSE;
            StringCchCopyW(st.error, ARRAYSIZE(st.error), L"Kulaklik bulunamadi.");
            StringCchCopyW(st.product, ARRAYSIZE(st.product), L"Razer BlackShark V2 HyperSpeed");
        }
    }
    {
        std::lock_guard<std::mutex> lock(g_stateMutex);
        g_battery = st;
    }
    g_ok.store(st.ok ? 1 : 0);
    g_percent.store(st.ok ? st.percent : -1);
    g_charging.store(st.charging ? 1 : 0);
}

// ----- GDI+ tray icon -----

Color LevelColor(int percent) {
    if (percent < 0) return Color(255, 120, 120, 125);
    if (percent <= 19) return Color(255, 255, 23, 68);
    if (percent <= 49) return Color(255, 255, 179, 0);
    return Color(255, 0, 230, 118);
}

void DrawLightning(Graphics& g, REAL cx, REAL cy, REAL s) {
    PointF pts[] = {
        {cx - s * 0.15f, cy - s * 0.55f},
        {cx + s * 0.20f, cy - s * 0.05f},
        {cx + s * 0.02f, cy - s * 0.05f},
        {cx + s * 0.25f, cy + s * 0.55f},
        {cx - s * 0.10f, cy + s * 0.05f},
        {cx + s * 0.08f, cy + s * 0.05f},
    };
    SolidBrush bolt(Color(255, 255, 255, 255));
    g.FillPolygon(&bolt, pts, 6);
}

HICON CreateBatteryIconGdiPlus(int percent, bool charging, bool ok) {
    const INT size = 32;
    Bitmap bmp(size, size, PixelFormat32bppARGB);
    Graphics g(&bmp);
    g.SetSmoothingMode(SmoothingModeAntiAlias);
    g.SetPixelOffsetMode(PixelOffsetModeHighQuality);
    g.Clear(Color(0, 0, 0, 0));

    const Color fill = LevelColor(ok ? percent : -1);
    const Color frame(255, 40, 40, 42);
    const Color bg(255, 22, 22, 24);

    // Battery body
    const REAL x = 3.0f, y = 8.0f, w = 22.0f, h = 16.0f, r = 3.0f;
    GraphicsPath body;
    body.AddArc(x, y, r * 2, r * 2, 180, 90);
    body.AddArc(x + w - r * 2, y, r * 2, r * 2, 270, 90);
    body.AddArc(x + w - r * 2, y + h - r * 2, r * 2, r * 2, 0, 90);
    body.AddArc(x, y + h - r * 2, r * 2, r * 2, 90, 90);
    body.CloseFigure();

    SolidBrush bgBrush(bg);
    g.FillPath(&bgBrush, &body);
    Pen border(frame, 1.5f);
    g.DrawPath(&border, &body);

    // Nipple
    SolidBrush nip(frame);
    g.FillRectangle(&nip, x + w, y + 4.5f, 3.0f, 7.0f);

    // Charge level fill
    if (ok && percent >= 0) {
        REAL inset = 2.5f;
        REAL maxInner = w - inset * 2;
        REAL fillW = maxInner * (Clamp(percent, 0, 100) / 100.0f);
        if (fillW > 0.5f) {
            SolidBrush level(fill);
            g.FillRectangle(&level, x + inset, y + inset, fillW, h - inset * 2);
        }
    }

    if (charging && ok)
        DrawLightning(g, x + w * 0.45f, y + h * 0.5f, 7.0f);

    // Tiny percent digits when not charging (readable on 32px)
    if (ok && !charging && percent >= 0) {
        FontFamily ff(L"Segoe UI");
        Font font(&ff, percent >= 100 ? 7.0f : 8.5f, FontStyleBold, UnitPixel);
        SolidBrush text(Color(255, 255, 255, 255));
        StringFormat fmt;
        fmt.SetAlignment(StringAlignmentCenter);
        fmt.SetLineAlignment(StringAlignmentCenter);
        wchar_t label[8]{};
        StringCchPrintfW(label, ARRAYSIZE(label), L"%d", percent);
        RectF tr(x, y, w, h);
        g.DrawString(label, -1, &font, tr, &fmt, &text);
    }

    HICON hIcon = nullptr;
    bmp.GetHICON(&hIcon);
    return hIcon;
}

void ApplyTrayVisual(BOOL showBalloon) {
    BatteryState snap{};
    {
        std::lock_guard<std::mutex> lock(g_stateMutex);
        snap = g_battery;
    }
    const int percent = g_percent.load();
    const bool charging = g_charging.load() != 0;
    const bool ok = g_ok.load() != 0;

    HICON next = CreateBatteryIconGdiPlus(percent, charging, ok);
    if (!next && g_appIcon)
        next = DuplicateIcon(nullptr, g_appIcon);

    if (g_icon) {
        DestroyIcon(g_icon);
        g_icon = nullptr;
    }
    g_icon = next;
    g_nid.hIcon = g_icon;

    wchar_t tip[128]{};
    if (ok && percent >= 0)
        StringCchPrintfW(tip, ARRAYSIZE(tip), L"BlackShark V2: %%%d%s", percent, charging ? L" (charging)" : L"");
    else
        StringCchPrintfW(tip, ARRAYSIZE(tip), L"BlackShark V2: %s", snap.error[0] ? snap.error : L"offline");
    tip[127] = 0;
    StringCchCopyW(g_nid.szTip, ARRAYSIZE(g_nid.szTip), tip);

    g_nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID | NIF_SHOWTIP;
    g_nid.guidItem = kTrayGuid;

    if (showBalloon && g_settings.notifyLow && ok && !charging && percent >= 0 &&
        percent <= g_settings.lowThreshold) {
        if (!g_notifiedLow) {
            g_notifiedLow = TRUE;
            g_nid.uFlags |= NIF_INFO;
            g_nid.dwInfoFlags = NIIF_WARNING;
            StringCchPrintfW(g_nid.szInfo, ARRAYSIZE(g_nid.szInfo), L"Low battery: %%%d", percent);
            StringCchCopyW(g_nid.szInfoTitle, ARRAYSIZE(g_nid.szInfoTitle), kAppTitle);
        }
    } else if (ok && percent >= g_settings.lowThreshold + 10) {
        g_notifiedLow = FALSE;
    }

    if (!Shell_NotifyIconW(NIM_MODIFY, &g_nid)) {
        Shell_NotifyIconW(NIM_ADD, &g_nid);
        Shell_NotifyIconW(NIM_SETVERSION, &g_nid);
    }
    g_nid.szInfo[0] = 0;
    g_nid.szInfoTitle[0] = 0;
}

void RequestPollAsync() {
    if (g_wakeEvent) SetEvent(g_wakeEvent);
}

void WorkerLoop() {
    while (!g_stopWorker.load()) {
        PollBattery();
        if (g_hwnd)
            PostMessageW(g_hwnd, WM_BATTERY_UPDATED, 0, 0);

        const DWORD waitMs = static_cast<DWORD>(Clamp(g_pollSeconds.load(), 15, 300)) * 1000u;
        if (!g_wakeEvent) break;
        DWORD wr = WaitForSingleObject(g_wakeEvent, waitMs);
        if (g_stopWorker.load()) break;
        if (wr == WAIT_OBJECT_0)
            ResetEvent(g_wakeEvent);
    }
}

void StartWorker() {
    g_stopWorker.store(0);
    g_wakeEvent = CreateEventW(nullptr, TRUE, TRUE, nullptr); // initially signaled => first poll soon
    g_worker = std::thread(WorkerLoop);
}

void StopWorker() {
    g_stopWorker.store(1);
    if (g_wakeEvent) SetEvent(g_wakeEvent);
    if (g_worker.joinable()) g_worker.join();
    if (g_wakeEvent) {
        CloseHandle(g_wakeEvent);
        g_wakeEvent = nullptr;
    }
}

// ----- Owner-draw dark menu -----

struct MenuItemData {
    UINT id;
    const wchar_t* text;
    BOOL bold;
    BOOL checkable;
};

void MeasureMenuItem(MEASUREITEMSTRUCT* mis) {
    if (mis->CtlType != ODT_MENU) return;
    if (mis->itemID == ID_SEP) {
        mis->itemWidth = 180;
        mis->itemHeight = 9;
        return;
    }
    mis->itemWidth = 220;
    mis->itemHeight = (mis->itemID == ID_HEADER) ? 34 : 30;
}

void DrawMenuItem(const DRAWITEMSTRUCT* dis) {
    if (dis->CtlType != ODT_MENU) return;
    HDC hdc = dis->hDC;
    RECT rc = dis->rcItem;
    const BOOL selected = (dis->itemState & ODS_SELECTED) != 0;
    const BOOL disabled = (dis->itemState & ODS_DISABLED) != 0 || (dis->itemState & ODS_GRAYED) != 0;

    const COLORREF bg = selected ? RGB(60, 60, 60) : RGB(30, 30, 30);
    HBRUSH brush = CreateSolidBrush(bg);
    FillRect(hdc, &rc, brush);
    DeleteObject(brush);

    if (dis->itemID == ID_SEP) {
        HPEN pen = CreatePen(PS_SOLID, 1, RGB(80, 80, 80));
        HGDIOBJ old = SelectObject(hdc, pen);
        int y = (rc.top + rc.bottom) / 2;
        MoveToEx(hdc, rc.left + 12, y, nullptr);
        LineTo(hdc, rc.right - 12, y);
        SelectObject(hdc, old);
        DeleteObject(pen);
        return;
    }

    wchar_t text[128]{};
    if (dis->itemID == ID_HEADER) {
        int p = g_percent.load();
        if (g_ok.load() && p >= 0)
            StringCchPrintfW(text, ARRAYSIZE(text), L"Razer BlackShark: %%%d", p);
        else
            StringCchCopyW(text, ARRAYSIZE(text), L"Razer BlackShark: --");
    } else if (dis->itemID == ID_REFRESH) {
        StringCchCopyW(text, ARRAYSIZE(text), L"Refresh Now");
    } else if (dis->itemID == ID_STARTUP) {
        StringCchPrintfW(text, ARRAYSIZE(text), L"%sRun at Startup",
                         g_settings.runAtStartup ? L"✓  " : L"    ");
    } else if (dis->itemID == ID_EXIT) {
        StringCchCopyW(text, ARRAYSIZE(text), L"Exit");
    }

    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, disabled ? RGB(140, 140, 140) : RGB(255, 255, 255));
    const int weight = (dis->itemID == ID_HEADER) ? FW_BOLD : FW_NORMAL;
    HFONT font = CreateFontW(-MulDiv(10, GetDeviceCaps(hdc, LOGPIXELSY), 72), 0, 0, 0, weight,
                             FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                             CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_SWISS, L"Segoe UI");
    HGDIOBJ oldFont = SelectObject(hdc, font);
    RECT textRc = rc;
    textRc.left += 14;
    DrawTextW(hdc, text, -1, &textRc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    SelectObject(hdc, oldFont);
    DeleteObject(font);
}

void ShowContextMenu() {
    POINT pt{};
    GetCursorPos(&pt);

    HMENU menu = CreatePopupMenu();
    AppendMenuW(menu, MF_OWNERDRAW | MF_DISABLED, ID_HEADER, nullptr);
    AppendMenuW(menu, MF_OWNERDRAW | MF_DISABLED, ID_SEP, nullptr);
    AppendMenuW(menu, MF_OWNERDRAW, ID_REFRESH, nullptr);
    AppendMenuW(menu, MF_OWNERDRAW, ID_STARTUP, nullptr);
    AppendMenuW(menu, MF_OWNERDRAW, ID_EXIT, nullptr);

    // Critical for Win32 tray menu responsiveness
    SetForegroundWindow(g_hwnd);
    UINT cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                              pt.x, pt.y, 0, g_hwnd, nullptr);
    PostMessageW(g_hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);

    if (cmd == ID_EXIT) {
        PostMessageW(g_hwnd, WM_CLOSE, 0, 0);
    } else if (cmd == ID_REFRESH) {
        RequestPollAsync();
    } else if (cmd == ID_STARTUP) {
        SetStartup(!g_settings.runAtStartup);
        SaveSettings();
    }
}

void AddTrayIcon() {
    ZeroMemory(&g_nid, sizeof(g_nid));
    g_nid.cbSize = sizeof(NOTIFYICONDATAW);
    g_nid.hWnd = g_hwnd;
    g_nid.uID = 1;
    g_nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID | NIF_SHOWTIP;
    g_nid.guidItem = kTrayGuid;
    g_nid.uCallbackMessage = WM_TRAY;
    g_nid.uVersion = NOTIFYICON_VERSION_4;
    StringCchCopyW(g_nid.szTip, ARRAYSIZE(g_nid.szTip), L"BlackShark V2");
    if (!g_icon)
        g_icon = CreateBatteryIconGdiPlus(-1, false, false);
    g_nid.hIcon = g_icon;

    Shell_NotifyIconW(NIM_DELETE, &g_nid);
    if (!Shell_NotifyIconW(NIM_ADD, &g_nid)) {
        g_nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        Shell_NotifyIconW(NIM_ADD, &g_nid);
    }
    Shell_NotifyIconW(NIM_SETVERSION, &g_nid);
}

void RemoveTrayIcon() {
    g_nid.uFlags = NIF_GUID;
    g_nid.guidItem = kTrayGuid;
    Shell_NotifyIconW(NIM_DELETE, &g_nid);
    g_nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    Shell_NotifyIconW(NIM_DELETE, &g_nid);
    if (g_icon) {
        DestroyIcon(g_icon);
        g_icon = nullptr;
    }
}

void RestartTimer() {
    KillTimer(g_hwnd, TIMER_POLL);
    // Lightweight UI heartbeat; real polling is on the worker thread.
    SetTimer(g_hwnd, TIMER_POLL, static_cast<UINT>(g_pollSeconds.load()) * 1000u, nullptr);
}

LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    if (msg == g_taskbarCreated) {
        AddTrayIcon();
        ApplyTrayVisual(FALSE);
        return 0;
    }

    switch (msg) {
    case WM_CREATE:
        g_hwnd = hwnd;
        AddTrayIcon();
        StartWorker();
        RestartTimer();
        return 0;
    case WM_BATTERY_UPDATED:
        ApplyTrayVisual(TRUE);
        return 0;
    case WM_TIMER:
        if (wParam == TIMER_POLL)
            RequestPollAsync();
        return 0;
    case WM_MEASUREITEM:
        MeasureMenuItem(reinterpret_cast<MEASUREITEMSTRUCT*>(lParam));
        return TRUE;
    case WM_DRAWITEM:
        DrawMenuItem(reinterpret_cast<const DRAWITEMSTRUCT*>(lParam));
        return TRUE;
    case WM_TRAY: {
        UINT event = LOWORD(lParam);
        if (event == WM_RBUTTONUP || event == WM_CONTEXTMENU) {
            ShowContextMenu();
        } else if (event == WM_LBUTTONDBLCLK || event == NIN_SELECT) {
            RequestPollAsync();
        }
        return 0;
    }
    case WM_CLOSE:
        StopWorker();
        RemoveTrayIcon();
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        KillTimer(hwnd, TIMER_POLL);
        PostQuitMessage(0);
        return 0;
    default:
        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }
}

void WriteOnceResult() {
    wchar_t path[MAX_PATH]{};
    GetModuleFileNameW(nullptr, path, MAX_PATH);
    wchar_t* slash = wcsrchr(path, L'\\');
    if (slash) slash[1] = 0;
    else path[0] = 0;
    StringCchCatW(path, ARRAYSIZE(path), L"battery-once.txt");

    PollBattery();
    wchar_t line[256]{};
    if (g_battery.ok) {
        StringCchPrintfW(line, ARRAYSIZE(line), L"OK %%%d (%s) %s\r\n",
                         g_battery.percent, g_battery.transport, g_battery.product);
    } else {
        StringCchPrintfW(line, ARRAYSIZE(line), L"FAIL %s\r\n", g_battery.error);
    }
    char utf8[512]{};
    WideCharToMultiByte(CP_UTF8, 0, line, -1, utf8, sizeof(utf8), nullptr, nullptr);
    HANDLE f = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (f != INVALID_HANDLE_VALUE) {
        DWORD wr = 0;
        WriteFile(f, utf8, static_cast<DWORD>(strlen(utf8)), &wr, nullptr);
        CloseHandle(f);
    }
}

} // namespace

int APIENTRY wWinMain(HINSTANCE hInst, HINSTANCE, LPWSTR cmdLine, int) {
    g_hInst = hInst;

    if (cmdLine && wcsstr(cmdLine, L"--once")) {
        WriteOnceResult();
        return g_battery.ok ? 0 : 1;
    }

    HANDLE mutex = CreateMutexW(nullptr, TRUE, L"Local\\BlackSharkBatteryTrayMutex");
    if (mutex && GetLastError() == ERROR_ALREADY_EXISTS) {
        HWND existing = FindWindowW(kWndClass, kAppTitle);
        if (existing) PostMessageW(existing, WM_TRAY, 0, MAKELPARAM(WM_LBUTTONDBLCLK, 0));
        return 0;
    }

    GdiplusStartupInput gdiInput;
    if (GdiplusStartup(&g_gdiplusToken, &gdiInput, nullptr) != Ok)
        return 1;

    InitCommonControls();
    LoadSettings();
    g_taskbarCreated = RegisterWindowMessageW(L"TaskbarCreated");
    g_appIcon = static_cast<HICON>(LoadImageW(hInst, MAKEINTRESOURCEW(IDI_APP), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE));

    WNDCLASSEXW wc{};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = WndProc;
    wc.hInstance = hInst;
    wc.lpszClassName = kWndClass;
    wc.hIcon = g_appIcon ? g_appIcon : LoadIconW(nullptr, IDI_APPLICATION);
    wc.hIconSm = wc.hIcon;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    if (!RegisterClassExW(&wc)) {
        GdiplusShutdown(g_gdiplusToken);
        return 1;
    }

    g_hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, kWndClass, kAppTitle, WS_POPUP,
                             0, 0, 0, 0, nullptr, nullptr, hInst, nullptr);
    if (!g_hwnd) {
        GdiplusShutdown(g_gdiplusToken);
        return 1;
    }
    ShowWindow(g_hwnd, SW_HIDE);

    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    GdiplusShutdown(g_gdiplusToken);
    if (mutex) CloseHandle(mutex);
    return static_cast<int>(msg.wParam);
}
