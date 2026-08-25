namespace BlackSharkBattery;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        if (args.Any(a => string.Equals(a, "--once", StringComparison.OrdinalIgnoreCase)))
        {
            var reading = BatteryService.Poll();
            string line = reading.Ok
                ? $"OK %{reading.Percent} ({reading.Transport}) {reading.Product}"
                : $"FAIL {reading.Error}";
            // WinExe has no console; write a tiny result file next to the exe / cwd.
            try { File.WriteAllText("battery-once.txt", line + Environment.NewLine); }
            catch { /* ignore */ }
            try { Console.WriteLine(line); } catch { /* no console */ }
            return;
        }

        ApplicationConfiguration.Initialize();
        Application.Run(new TrayApplicationContext());
    }
}
