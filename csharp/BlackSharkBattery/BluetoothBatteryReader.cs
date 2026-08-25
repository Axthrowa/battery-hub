using System.Runtime.InteropServices;
using System.Text;

namespace BlackSharkBattery;

/// <summary>
/// Fallback: Windows PnP / HFP battery property for paired Bluetooth headsets.
/// </summary>
internal static class BluetoothBatteryReader
{
    private static readonly string[] NameHints =
    [
        "blackshark v2 hs",
        "blackshark v2 hyperspeed",
        "razer blackshark v2",
        "blackshark v2 hs bt",
    ];

    // Undocumented HFP battery DEVPKEY used by several Windows tray monitors.
    private static readonly DEVPROPKEY BatteryKey = new(
        new Guid(0x104EA319, 0x6EE2, 0x4701, 0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5), 2);

    private static readonly DEVPROPKEY FriendlyName = new(
        new Guid(0xA45C254E, 0xDF1C, 0x4EFD, 0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0), 14);

    private static readonly Guid BluetoothClass = new(0xE0CBF06C, 0xCD8B, 0x4647, 0xBB, 0x8A, 0x26, 0x3B, 0x43, 0xF0, 0xF9, 0x74);

    private const int DigcfPresent = 0x00000002;
    private const int DevpropTypeUint32 = 0x00000007;
    private const int DevpropTypeByte = 0x00000003;
    private const int DevpropTypeString = 0x00000012;
    private const int DevpropTypeMask = 0x00000FFF;

    public static BatteryReading? Read()
    {
        IntPtr devs;
        var classGuid = BluetoothClass;
        devs = SetupDiGetClassDevsW(ref classGuid, IntPtr.Zero, IntPtr.Zero, DigcfPresent);
        if (devs == IntPtr.Zero || devs == new IntPtr(-1))
            return null;

        try
        {
            var info = new SP_DEVINFO_DATA { cbSize = Marshal.SizeOf<SP_DEVINFO_DATA>() };
            for (uint i = 0; SetupDiEnumDeviceInfo(devs, i, ref info); i++)
            {
                string? name = ReadString(devs, ref info, FriendlyName);
                if (string.IsNullOrWhiteSpace(name) || !NameMatches(name))
                    continue;

                int? battery = ReadUInt(devs, ref info, BatteryKey);
                if (battery is null)
                    continue;

                return new BatteryReading
                {
                    Ok = true,
                    Percent = Math.Clamp(battery.Value, 0, 100),
                    Charging = false,
                    Transport = "Bluetooth",
                    Product = name,
                };
            }
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(devs);
        }

        return null;
    }

    private static bool NameMatches(string name)
    {
        var lower = name.ToLowerInvariant();
        return NameHints.Any(h => lower.Contains(h, StringComparison.Ordinal));
    }

    private static string? ReadString(IntPtr devs, ref SP_DEVINFO_DATA info, DEVPROPKEY key)
    {
        if (!TryReadProp(devs, ref info, key, out uint type, out byte[] raw))
            return null;
        if ((type & DevpropTypeMask) != DevpropTypeString)
            return null;
        return Encoding.Unicode.GetString(raw).TrimEnd('\0');
    }

    private static int? ReadUInt(IntPtr devs, ref SP_DEVINFO_DATA info, DEVPROPKEY key)
    {
        if (!TryReadProp(devs, ref info, key, out uint type, out byte[] raw) || raw.Length == 0)
            return null;
        int kind = (int)(type & DevpropTypeMask);
        if (kind is DevpropTypeByte or DevpropTypeUint32 || raw.Length >= 1)
        {
            if (raw.Length >= 4)
                return BitConverter.ToInt32(raw, 0);
            return raw[0];
        }
        return null;
    }

    private static bool TryReadProp(IntPtr devs, ref SP_DEVINFO_DATA info, DEVPROPKEY key, out uint type, out byte[] raw)
    {
        type = 0;
        raw = [];
        uint needed = 0;
        SetupDiGetDevicePropertyW(devs, ref info, ref key, out type, IntPtr.Zero, 0, ref needed, 0);
        if (needed == 0)
            return false;

        IntPtr buf = Marshal.AllocHGlobal((int)needed);
        try
        {
            if (!SetupDiGetDevicePropertyW(devs, ref info, ref key, out type, buf, needed, ref needed, 0))
                return false;
            raw = new byte[needed];
            Marshal.Copy(buf, raw, 0, (int)needed);
            return true;
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct SP_DEVINFO_DATA
    {
        public int cbSize;
        public Guid ClassGuid;
        public uint DevInst;
        public IntPtr Reserved;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct DEVPROPKEY
    {
        public Guid fmtid;
        public uint pid;
        public DEVPROPKEY(Guid fmtid, uint pid) { this.fmtid = fmtid; this.pid = pid; }
    }

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SetupDiGetClassDevsW(ref Guid classGuid, IntPtr enumerator, IntPtr hwndParent, int flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiEnumDeviceInfo(IntPtr deviceInfoSet, uint memberIndex, ref SP_DEVINFO_DATA deviceInfoData);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDevicePropertyW(
        IntPtr deviceInfoSet,
        ref SP_DEVINFO_DATA deviceInfoData,
        ref DEVPROPKEY propertyKey,
        out uint propertyType,
        IntPtr propertyBuffer,
        uint propertyBufferSize,
        ref uint requiredSize,
        uint flags);

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr deviceInfoSet);
}
