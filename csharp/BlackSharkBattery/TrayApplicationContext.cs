namespace BlackSharkBattery;

internal sealed class TrayApplicationContext : ApplicationContext
{
    private readonly NotifyIcon _tray;
    private readonly System.Windows.Forms.Timer _timer;
    private readonly ContextMenuStrip _menu;
    private Icon? _currentIcon;
    private BatteryReading _last = new() { Ok = false, Error = "Cihaz aranıyor…" };

    public TrayApplicationContext()
    {
        _menu = new ContextMenuStrip();
        _menu.Items.Add("Şimdi yenile", null, (_, _) => RefreshBattery());
        _menu.Items.Add(new ToolStripSeparator());
        _menu.Items.Add("Çıkış", null, (_, _) => Exit());

        _tray = new NotifyIcon
        {
            Visible = true,
            Text = "Razer BlackShark",
            ContextMenuStrip = _menu,
            Icon = TrayIconFactory.Create(null, false, true),
        };
        _currentIcon = _tray.Icon;
        _tray.DoubleClick += (_, _) => RefreshBattery();

        _timer = new System.Windows.Forms.Timer { Interval = 60_000 };
        _timer.Tick += (_, _) => RefreshBattery();
        _timer.Start();

        // Immediate first reading (HID query is ~1s; keeps code on the UI thread).
        RefreshBattery();
    }

    private void RefreshBattery()
    {
        Apply(BatteryService.Poll());
    }

    private void Apply(BatteryReading reading)
    {
        _last = reading;
        var old = _currentIcon;
        var next = TrayIconFactory.Create(reading.Percent, reading.Charging, !reading.Ok);
        _tray.Icon = next;
        _currentIcon = next;
        old?.Dispose();

        _tray.Text = BuildTooltip(reading);
    }

    private static string BuildTooltip(BatteryReading reading)
    {
        // NotifyIcon.Text max length is 63 chars on Windows.
        string tip;
        if (reading.Ok && reading.Percent is int p)
        {
            string charge = reading.Charging ? " şarj" : "";
            tip = $"Razer BlackShark: %{p}{charge}";
            if (!string.IsNullOrWhiteSpace(reading.Transport))
                tip += $" · {reading.Transport}";
        }
        else
        {
            tip = reading.Error is { Length: > 0 }
                ? $"Razer BlackShark: {reading.Error}"
                : "Razer BlackShark: bağlı değil";
        }

        return tip.Length <= 63 ? tip : tip[..63];
    }

    private void Exit()
    {
        _timer.Stop();
        _tray.Visible = false;
        _tray.Dispose();
        _currentIcon?.Dispose();
        _menu.Dispose();
        ExitThread();
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            _timer.Dispose();
            _tray.Dispose();
            _currentIcon?.Dispose();
            _menu.Dispose();
        }
        base.Dispose(disposing);
    }
}
