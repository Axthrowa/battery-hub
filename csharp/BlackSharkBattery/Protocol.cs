namespace BlackSharkBattery;

/// <summary>
/// MediaTek / BlackShark V2 HyperSpeed vendor HID framing (report 0x02).
/// Proven on Windows against dongle PID 0x0565, usage page 0xFF14.
/// </summary>
internal static class Protocol
{
    public const int RazerVid = 0x1532;
    public const int PidDongle = 0x0565;
    public const int PidWired = 0x056E;

    public const int PreferredUsagePage = 0xFF14;
    public const int ReportLen = 64;
    public const byte ReportId = 0x02;
    public const byte RfWakeReportId = 0x05;
    public const byte Channel = 0x60;
    public const int CrcIndex = 62;
    public const byte ClassHeadset = 0x80;

    public const byte CmdBattery = 0x21;
    public const byte CmdCharging = 0x2A;
    public const byte CmdLink = 0x20;

    public const int ReplyTimeoutMs = 1200;

    public static byte XorChecksum(ReadOnlySpan<byte> buf)
    {
        byte crc = 0;
        for (int i = 0; i < CrcIndex; i++)
            crc ^= buf[i];
        return crc;
    }

    public static byte[] BuildQuery(byte cmd, bool dongle, byte cls = ClassHeadset)
    {
        var buf = new byte[ReportLen];
        buf[0] = ReportId;
        buf[2] = Channel;
        buf[6] = 0x04;
        buf[10] = cmd;
        buf[12] = 0x00;
        if (dongle)
        {
            buf[9] = cls;
            buf[CrcIndex] = XorChecksum(buf);
        }
        return buf;
    }

    public static bool TryParseReply(ReadOnlySpan<byte> data, byte expectedCmd, out int value)
    {
        value = 0;
        if (data.Length <= 13)
            return false;

        ReadOnlySpan<byte> payload = data;
        if (data[0] == ReportId)
            payload = data;
        else if (data.Length > 14 && data[1] == ReportId)
            payload = data[1..];
        else if (data[0] != ReportId)
            return false;

        if (payload.Length <= 13)
            return false;
        if (payload[10] != expectedCmd)
            return false;
        if (payload[11] != 0x01)
            return false;

        value = payload[13];
        return true;
    }
}
