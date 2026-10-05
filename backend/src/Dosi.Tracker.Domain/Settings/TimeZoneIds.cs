using System;
using System.Diagnostics.CodeAnalysis;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.Settings;

/// <summary>Validation of the time zone ids users and owners choose. Zones are stored as IANA ids ("Asia/Dhaka"),
/// the form browsers and both desktop agents use.</summary>
public static class TimeZoneIds
{
    /// <summary>Longest id kept (same bound as the agents' device time zone).</summary>
    public const int MaxLength = ActivityConsts.MaxTimeZoneLength;

    /// <summary>
    /// Accepts any id <see cref="TimeZoneInfo.TryFindSystemTimeZoneById"/> knows and returns its IANA form: an IANA
    /// id is kept as given (trimmed), a Windows id ("Bangladesh Standard Time") is converted. False for an unknown
    /// id, a blank one, one longer than <see cref="MaxLength"/>, or a zone without an IANA equivalent.
    /// </summary>
    public static bool TryNormalize(string? timeZoneId, [NotNullWhen(true)] out string? ianaId)
    {
        ianaId = null;
        var id = timeZoneId?.Trim();
        if (string.IsNullOrEmpty(id) || id.Length > MaxLength ||
            !TimeZoneInfo.TryFindSystemTimeZoneById(id, out var zone))
        {
            return false;
        }

        // Checked first so an id that is both IANA and Windows ("UTC") stays as given on every OS.
        if (TimeZoneInfo.TryConvertIanaIdToWindowsId(id, out _))
        {
            ianaId = id;
            return true;
        }

        if (zone.HasIanaId)
        {
            ianaId = zone.Id;
            return true;
        }

        if (TimeZoneInfo.TryConvertWindowsIdToIanaId(zone.Id, out var converted))
        {
            ianaId = converted;
            return true;
        }

        return false;
    }
}
