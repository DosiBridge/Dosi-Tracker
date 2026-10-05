using System.Globalization;

namespace Dosi.Tracker.Settings;

/// <summary>How the tracker's setting values are written as (and read back from) setting strings.</summary>
internal static class SettingValueFormat
{
    public static string Write(int value)
    {
        return value.ToString(CultureInfo.InvariantCulture);
    }

    public static string Write(bool value)
    {
        return value ? "true" : "false";
    }

    /// <summary>The stored number, or <paramref name="fallback"/> when it is missing or unreadable.</summary>
    public static int ReadInt(string? value, int fallback)
    {
        return int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var number) ? number : fallback;
    }

    /// <summary>The stored flag (case-insensitive), or <paramref name="fallback"/> when it is missing or unreadable.</summary>
    public static bool ReadBool(string? value, bool fallback)
    {
        return bool.TryParse(value, out var flag) ? flag : fallback;
    }
}
