namespace BlackSharkBattery;

internal static class BatteryService
{
    public static BatteryReading Poll()
    {
        try
        {
            var hid = HidBatteryReader.Read();
            if (hid is { Ok: true })
                return hid;

            var bt = BluetoothBatteryReader.Read();
            if (bt is { Ok: true })
                return bt;

            if (hid is not null)
                return hid;

            return new BatteryReading
            {
                Ok = false,
                Error = "Kulaklık bulunamadı. 2.4 GHz dongle veya Bluetooth bağlantısını kontrol edin.",
            };
        }
        catch (Exception ex)
        {
            return new BatteryReading
            {
                Ok = false,
                Error = ex.Message,
            };
        }
    }
}
