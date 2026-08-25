using HidSharp;

namespace BlackSharkBattery;

internal sealed class BatteryReading
{
    public bool Ok { get; init; }
    public int? Percent { get; init; }
    public bool Charging { get; init; }
    public string Transport { get; init; } = "";
    public string Product { get; init; } = "Razer BlackShark V2 HyperSpeed";
    public string? Error { get; init; }
}

internal static class HidBatteryReader
{
    public static BatteryReading? Read()
    {
        var devices = DeviceList.Local.GetHidDevices(Protocol.RazerVid)
            .Where(d => d.ProductID is Protocol.PidDongle or Protocol.PidWired)
            .OrderByDescending(Score)
            .ToList();

        if (devices.Count == 0)
            return null;

        HidDevice? last = null;
        foreach (var device in devices)
        {
            last = device;
            try
            {
                if (device.GetMaxInputReportLength() < 16)
                    continue;
            }
            catch
            {
                continue;
            }

            try
            {
                var result = ReadDevice(device);
                if (result is { Ok: true })
                    return result;
            }
            catch
            {
                // try next HID collection
            }
        }

        bool dongle = last is null || last.ProductID == Protocol.PidDongle;
        return new BatteryReading
        {
            Ok = false,
            Transport = dongle ? "2.4 GHz" : "USB",
            Product = SafeProduct(last) ?? "Razer BlackShark V2 HyperSpeed",
            Error = "Dongle bulundu ama kulaklık yanıt vermedi.",
        };
    }

    private static int Score(HidDevice d)
    {
        int score = d.ProductID == Protocol.PidDongle ? 1 : 0;
        string path = d.DevicePath ?? "";
        // Windows splits MI_03 into Col01=FF13, Col04=FF14 (working battery channel).
        if (path.Contains("Col04", StringComparison.OrdinalIgnoreCase))
            score += 100;
        else if (path.Contains("MI_03", StringComparison.OrdinalIgnoreCase))
            score += 20;
        return score;
    }

    private static string? SafeProduct(HidDevice? d)
    {
        if (d is null) return null;
        try
        {
            var name = d.GetProductName();
            return string.IsNullOrWhiteSpace(name) ? null : name;
        }
        catch
        {
            return null;
        }
    }

    private static BatteryReading? ReadDevice(HidDevice device)
    {
        if (!device.TryOpen(out var stream))
            return null;

        using (stream)
        {
            stream.ReadTimeout = 250;
            stream.WriteTimeout = 1000;

            Drain(stream);
            TryWake(stream);

            bool dongle = device.ProductID == Protocol.PidDongle;
            _ = QueryByte(stream, Protocol.BuildQuery(Protocol.CmdLink, dongle), Protocol.CmdLink, 500);

            int? percent = QueryByte(
                stream,
                Protocol.BuildQuery(Protocol.CmdBattery, dongle),
                Protocol.CmdBattery,
                Protocol.ReplyTimeoutMs);

            if (percent is null && dongle)
            {
                percent = QueryByte(
                    stream,
                    Protocol.BuildQuery(Protocol.CmdBattery, dongle: false),
                    Protocol.CmdBattery,
                    800);
            }

            if (percent is null)
                return null;

            int? charging = QueryByte(
                stream,
                Protocol.BuildQuery(Protocol.CmdCharging, dongle),
                Protocol.CmdCharging,
                800);

            return new BatteryReading
            {
                Ok = true,
                Percent = Math.Clamp(percent.Value, 0, 100),
                Charging = charging is > 0,
                Transport = dongle ? "2.4 GHz" : "USB",
                Product = SafeProduct(device) ?? "Razer BlackShark V2 HyperSpeed",
            };
        }
    }

    private static void TryWake(HidStream stream)
    {
        try
        {
            var wake = new byte[Protocol.ReportLen];
            wake[0] = Protocol.RfWakeReportId;
            stream.Write(wake);
            Thread.Sleep(40);
        }
        catch
        {
            try
            {
                stream.Write([Protocol.RfWakeReportId, 0x00]);
                Thread.Sleep(40);
            }
            catch
            {
                // optional
            }
        }
    }

    private static void Drain(HidStream stream)
    {
        var buf = new byte[Math.Max(Protocol.ReportLen, 65)];
        var old = stream.ReadTimeout;
        stream.ReadTimeout = 1;
        try
        {
            for (int i = 0; i < 32; i++)
            {
                try { _ = stream.Read(buf, 0, buf.Length); }
                catch (TimeoutException) { break; }
                catch (IOException) { break; }
            }
        }
        finally
        {
            stream.ReadTimeout = old;
        }
    }

    private static int? QueryByte(HidStream stream, byte[] report, byte cmd, int timeoutMs)
    {
        Drain(stream);
        try { stream.Write(report); }
        catch { return null; }

        var buf = new byte[Math.Max(Protocol.ReportLen, 65)];
        var deadline = Environment.TickCount64 + timeoutMs;
        stream.ReadTimeout = 250;
        while (Environment.TickCount64 < deadline)
        {
            int n;
            try { n = stream.Read(buf, 0, buf.Length); }
            catch (TimeoutException) { continue; }
            catch (IOException) { break; }

            if (n <= 0) continue;
            if (Protocol.TryParseReply(buf.AsSpan(0, n), cmd, out int value))
                return value;
        }

        return null;
    }
}
