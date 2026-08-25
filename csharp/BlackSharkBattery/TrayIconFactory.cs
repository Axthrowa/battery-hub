namespace BlackSharkBattery;

internal static class TrayIconFactory
{
    public static Icon Create(int? percent, bool charging, bool missing)
    {
        const int size = 32;
        using var bmp = new Bitmap(size, size);
        using var g = Graphics.FromImage(bmp);
        g.Clear(Color.Transparent);
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        g.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;

        Color color = missing || percent is null
            ? Color.FromArgb(150, 150, 155)
            : charging
                ? Color.FromArgb(56, 176, 222)
                : percent <= 15
                    ? Color.FromArgb(220, 70, 70)
                    : percent <= 35
                        ? Color.FromArgb(230, 170, 50)
                        : Color.FromArgb(70, 190, 110);

        using (var fill = new SolidBrush(Color.FromArgb(240, 20, 22, 26)))
        using (var pen = new Pen(color, 2f))
        {
            var rect = new Rectangle(1, 1, size - 3, size - 3);
            g.FillRectangle(fill, rect);
            g.DrawRectangle(pen, rect);
        }

        string text = missing || percent is null ? "?" : percent >= 100 ? "100" : percent.Value.ToString();
        float fontSize = text.Length >= 3 ? 9f : text.Length == 1 ? 14f : 12f;
        using var font = new Font("Segoe UI", fontSize, FontStyle.Bold, GraphicsUnit.Pixel);
        using var textBrush = new SolidBrush(color);
        var sizeF = g.MeasureString(text, font);
        g.DrawString(text, font, textBrush, (size - sizeF.Width) / 2f, (size - sizeF.Height) / 2f - 0.5f);

        IntPtr hIcon = bmp.GetHicon();
        try
        {
            using var tmp = Icon.FromHandle(hIcon);
            return (Icon)tmp.Clone();
        }
        finally
        {
            DestroyIcon(hIcon);
        }
    }

    [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Auto)]
    private static extern bool DestroyIcon(IntPtr handle);
}
