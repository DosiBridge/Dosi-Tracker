using System;
using System.Globalization;
using Dosi.Tracker.Reporting;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>
/// How every value in a context bundle is written: the server formats each number, duration, time and date once, and
/// the model only ever refers to it by its fact placeholder. English and culture-invariant, so the same data always
/// gives the same text (and the same cache key).
/// </summary>
public static class DisplayFormat
{
    public const string NotMeasured = "not measured";
    public const string NoChange = "no change";

    private static readonly CultureInfo Culture = CultureInfo.InvariantCulture;

    /// <summary>"6 h 52 min", "45 min", "2 h", "0 min", or "less than 1 min" for a positive value under half a
    /// minute; rounded to the minute. A negative value reads as 0.</summary>
    public static string Duration(double minutes)
    {
        if (double.IsNaN(minutes) || minutes <= 0)
        {
            return "0 min";
        }

        var whole = (long)Math.Round(minutes, MidpointRounding.AwayFromZero);
        if (whole == 0)
        {
            return "less than 1 min";
        }

        var hours = whole / 60;
        var rest = whole % 60;
        if (hours == 0)
        {
            return string.Create(Culture, $"{rest} min");
        }

        return rest == 0
            ? string.Create(Culture, $"{hours} h")
            : string.Create(Culture, $"{hours} h {rest} min");
    }

    /// <summary>"71%" (rounded to a whole percent), or "not measured" for null, which is never the same as 0.</summary>
    public static string Percent(double? value)
    {
        if (value is not { } percent || double.IsNaN(percent))
        {
            return NotMeasured;
        }

        return string.Create(Culture, $"{Math.Round(percent, MidpointRounding.AwayFromZero):0}%");
    }

    /// <summary>The instant on the zone's clock, as "09:12".</summary>
    public static string LocalTime(DateTime instantUtc, TimeZoneInfo zone)
    {
        return ToLocal(instantUtc, zone).ToString("HH:mm", Culture);
    }

    /// <summary>A local calendar day as "Fri 24 Oct", or "Fri 24 Oct 2025" with <paramref name="withYear"/> (see
    /// <see cref="NeedsYear"/>).</summary>
    public static string Day(DateTime localDate, bool withYear = false)
    {
        return localDate.ToString(withYear ? "ddd d MMM yyyy" : "ddd d MMM", Culture);
    }

    /// <summary>True when a range's days must carry their year: the range spans more than one calendar year, or is
    /// not in the current one.</summary>
    public static bool NeedsYear(DateTime firstLocalDay, DateTime lastLocalDay, DateTime todayLocal)
    {
        return firstLocalDay.Year != lastLocalDay.Year || firstLocalDay.Year != todayLocal.Year;
    }

    /// <summary>"Asia/Dhaka (UTC+06:00)": the zone's id and its offset at <paramref name="atUtc"/> (offsets change with
    /// daylight saving time).</summary>
    public static string Zone(TimeZoneInfo zone, DateTime atUtc)
    {
        var offset = zone.GetUtcOffset(ReportCalendar.AsUtc(atUtc));
        var sign = offset < TimeSpan.Zero ? "-" : "+";
        var magnitude = offset.Duration();
        return string.Create(Culture, $"{zone.Id} (UTC{sign}{magnitude.Hours:00}:{magnitude.Minutes:00})");
    }

    /// <summary>A change of duration: "+42 min", "-1 h 5 min", or "no change" when it rounds to 0 minutes.</summary>
    public static string Delta(double minutes)
    {
        if (double.IsNaN(minutes) || Math.Round(Math.Abs(minutes), MidpointRounding.AwayFromZero) == 0)
        {
            return NoChange;
        }

        return (minutes > 0 ? "+" : "-") + Duration(Math.Abs(minutes));
    }

    /// <summary>The relative change from <paramref name="previous"/> to <paramref name="current"/>: "+12%", "-30%",
    /// or "no change"; null when it cannot be stated (a value not measured, or nothing to compare with).</summary>
    public static string? DeltaPercent(double? current, double? previous)
    {
        if (current is not { } now || previous is not { } before || before <= 0 || double.IsNaN(now) || double.IsNaN(before))
        {
            return null;
        }

        var change = Math.Round((now - before) / before * 100, MidpointRounding.AwayFromZero);
        if (change == 0)
        {
            return NoChange;
        }

        return string.Create(Culture, $"{(change > 0 ? "+" : "-")}{Math.Abs(change):0}%");
    }

    private static DateTime ToLocal(DateTime instant, TimeZoneInfo zone)
    {
        return TimeZoneInfo.ConvertTimeFromUtc(ReportCalendar.AsUtc(instant), zone);
    }
}
