using System;
using System.Linq;
using System.Text;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Normalizes the optional context that desktop agents send with a block or a heartbeat (shared contract §1, §3).
/// It never throws: a value that cannot be used is stored as null, so these fields never cost an upload.
/// </summary>
internal static class AgentContextNormalizer
{
    /// <summary>The largest device clock skew stored: ±10 days.</summary>
    public const int MaxClockSkewSeconds = 10 * 24 * 60 * 60;

    /// <summary>Control characters removed, surrounding whitespace trimmed and the rest cut to
    /// <paramref name="maxLength"/>; null when nothing is left.</summary>
    public static string? CleanText(string? value, int maxLength)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        var text = value.Any(char.IsControl)
            ? new string(value.Where(c => !char.IsControl(c)).ToArray()).Trim()
            : value.Trim();
        if (text.Length > maxLength)
        {
            text = Truncate(text, maxLength)!.TrimEnd();
        }

        return text.Length == 0 ? null : text;
    }

    /// <summary>Cuts <paramref name="value"/> to at most <paramref name="maxLength"/> characters without splitting a
    /// surrogate pair (half a pair cannot be stored as UTF-8).</summary>
    public static string? Truncate(string? value, int maxLength)
    {
        if (value == null || value.Length <= maxLength)
        {
            return value;
        }

        var length = maxLength > 0 && char.IsHighSurrogate(value[maxLength - 1]) ? maxLength - 1 : maxLength;
        return value[..length];
    }

    /// <summary>A task note: control characters removed, every run of whitespace (line breaks and tabs included)
    /// collapsed to one space, trimmed and cut to <paramref name="maxLength"/>; null when nothing is left.</summary>
    public static string? CleanNote(string? value, int maxLength)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        var builder = new StringBuilder(value.Length);
        var pendingSpace = false;
        foreach (var c in value)
        {
            if (char.IsWhiteSpace(c))
            {
                pendingSpace = builder.Length > 0;
                continue;
            }

            if (char.IsControl(c))
            {
                continue;
            }

            if (pendingSpace)
            {
                builder.Append(' ');
                pendingSpace = false;
            }

            builder.Append(c);
        }

        var note = builder.Length > maxLength ? Truncate(builder.ToString(), maxLength)!.TrimEnd() : builder.ToString();
        return note.Length == 0 ? null : note;
    }

    /// <summary>"windows" or "macos" in lower case; anything else is null.</summary>
    public static string? NormalizePlatform(string? value) => FindKnown(value, ActivityConsts.Platforms);

    /// <summary>One of <see cref="ActivityConsts.EndReasons"/> in lower case; anything else is null.</summary>
    public static string? NormalizeEndReason(string? value) => FindKnown(value, ActivityConsts.EndReasons);

    /// <summary>A random per-install id; the empty GUID means "not sent".</summary>
    public static Guid? NormalizeDeviceId(Guid? value) => value == Guid.Empty ? null : value;

    /// <summary>The UTC offset when it lies within UTC-14:00 .. UTC+14:00; null otherwise.</summary>
    public static int? NormalizeUtcOffset(int? minutes) =>
        minutes is >= ActivityConsts.MinUtcOffsetMinutes and <= ActivityConsts.MaxUtcOffsetMinutes ? minutes : null;

    /// <summary>
    /// An IANA time zone id the server knows (<see cref="TimeZoneInfo.TryFindSystemTimeZoneById"/>), trimmed;
    /// null for an unknown, blank or over-long id. A Windows zone id is stored as its IANA equivalent, so reports
    /// see one kind of id whatever the agent's platform.
    /// </summary>
    public static string? NormalizeTimeZone(string? value)
    {
        var id = value?.Trim();
        if (string.IsNullOrEmpty(id) || id.Length > ActivityConsts.MaxTimeZoneLength)
        {
            return null;
        }

        if (!TimeZoneInfo.TryConvertIanaIdToWindowsId(id, out _) &&
            TimeZoneInfo.TryConvertWindowsIdToIanaId(id, out var ianaId))
        {
            id = ianaId;
        }

        return TimeZoneInfo.TryFindSystemTimeZoneById(id, out _) ? id : null;
    }

    /// <summary>Server time minus the device's clock when it sent the upload, in whole seconds (positive: the
    /// device clock is behind), clamped to ±<see cref="MaxClockSkewSeconds"/>; null when the agent did not say.</summary>
    public static int? ClockSkewSeconds(DateTime serverNowUtc, DateTime? sentAt)
    {
        if (!sentAt.HasValue)
        {
            return null;
        }

        var seconds = Math.Round((AsUtc(serverNowUtc) - AsUtc(sentAt.Value)).TotalSeconds, MidpointRounding.AwayFromZero);
        return (int)Math.Clamp(seconds, -MaxClockSkewSeconds, MaxClockSkewSeconds);
    }

    /// <summary>Times are UTC; an unspecified kind is read as UTC.</summary>
    public static DateTime AsUtc(DateTime value) => value.Kind switch
    {
        DateTimeKind.Local => value.ToUniversalTime(),
        DateTimeKind.Unspecified => DateTime.SpecifyKind(value, DateTimeKind.Utc),
        _ => value
    };

    private static string? FindKnown(string? value, string[] known)
    {
        var text = value?.Trim();
        return string.IsNullOrEmpty(text)
            ? null
            : known.FirstOrDefault(k => string.Equals(k, text, StringComparison.OrdinalIgnoreCase));
    }
}
