using System;
using System.Collections.Generic;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The calendar the day- and hour-based reports read time on: the requested IANA time zone (UTC when it is
/// absent or unknown) and the day basis.
/// <para>
/// Day rule: a block belongs, whole, to the local day of its start (StartedAt) — it is never split across
/// days, so a block that runs past midnight counts on the day it began. With the "zone" basis that local day is
/// read on the requested zone's clock (DST-aware); with the "member" basis it is read on the clock of the device
/// that tracked the block: its UtcOffsetMinutes at StartedAt, else its IANA TimeZone, else the requested zone.
/// The hourly report is the one exception to "whole": it splits blocks across clock-hour boundaries.
/// </para>
/// </summary>
public sealed class ReportCalendar
{
    private readonly Dictionary<string, TimeZoneInfo?> _deviceZones = new(StringComparer.Ordinal);

    public ReportCalendar(TimeZoneInfo zone, bool memberBasis)
    {
        Zone = zone;
        MemberBasis = memberBasis;
        ZoneClock = LocalClock.Of(zone);
    }

    /// <summary>The requested time zone: the report's calendar days and the attendance rules use it.</summary>
    public TimeZoneInfo Zone { get; }

    /// <summary>True when blocks are placed on days by the clock of the device that tracked them.</summary>
    public bool MemberBasis { get; }

    /// <summary>The requested zone's clock.</summary>
    public LocalClock ZoneClock { get; }

    public static ReportCalendar For(GetReportSummaryInput input)
    {
        return new ReportCalendar(ResolveZone(input.TimeZone), IsMemberBasis(input.DayBasis));
    }

    /// <summary>The time zone with this id (IANA, e.g. "America/New_York"); UTC when the id is blank or unknown.</summary>
    public static TimeZoneInfo ResolveZone(string? id)
    {
        return TryResolveZone(id) ?? TimeZoneInfo.Utc;
    }

    /// <summary>True for <see cref="ReportDayBasis.Member"/> (any casing); everything else is the zone basis.</summary>
    public static bool IsMemberBasis(string? dayBasis)
    {
        return string.Equals(dayBasis?.Trim(), ReportDayBasis.Member, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>The clock a block's day (and hours) are read on — see the class remarks for the day rule.</summary>
    public LocalClock ClockFor(string? blockTimeZone, int? blockUtcOffsetMinutes)
    {
        if (!MemberBasis)
        {
            return ZoneClock;
        }

        if (blockUtcOffsetMinutes is >= ActivityConsts.MinUtcOffsetMinutes and <= ActivityConsts.MaxUtcOffsetMinutes)
        {
            return LocalClock.Fixed(TimeSpan.FromMinutes(blockUtcOffsetMinutes.Value));
        }

        var deviceZone = DeviceZone(blockTimeZone);
        return deviceZone == null ? ZoneClock : LocalClock.Of(deviceZone);
    }

    /// <summary>Every local date (00:00, Kind Unspecified) of the requested zone that overlaps [fromUtc, toUtc),
    /// oldest first: the continuous axis of the day-based reports. Every block of the range starts on one of
    /// these days in the zone's calendar.</summary>
    public List<DateTime> DaysOverlapping(DateTime fromUtc, DateTime toUtc)
    {
        var days = new List<DateTime>();
        var last = ZoneClock.ToLocal(toUtc.AddTicks(-1)).Date;
        for (var day = ZoneClock.ToLocal(fromUtc).Date; day <= last; day = day.AddDays(1))
        {
            days.Add(day);
        }

        return days;
    }

    /// <summary>The first instant (UTC) of a local date of the requested zone. Usually its midnight; when the clocks
    /// skip midnight (DST starting at 00:00) the day starts at the jump, and when midnight occurs twice (DST ending
    /// at 00:00) at its first occurrence.</summary>
    public DateTime StartOfDayUtc(DateTime localDate)
    {
        var midnight = DateTime.SpecifyKind(localDate.Date, DateTimeKind.Unspecified);

        if (Zone.IsInvalidTime(midnight))
        {
            // Clocks jumped over midnight: the day began at the jump, which the previous day's offset dates.
            var offsetBefore = Zone.GetUtcOffset(midnight.AddDays(-1));
            return DateTime.SpecifyKind(midnight - offsetBefore, DateTimeKind.Utc);
        }

        if (Zone.IsAmbiguousTime(midnight))
        {
            // Midnight happened twice: the day began at the first one, under the larger (pre-transition) offset.
            var earliest = TimeSpan.MinValue;
            foreach (var offset in Zone.GetAmbiguousTimeOffsets(midnight))
            {
                earliest = offset > earliest ? offset : earliest;
            }

            return DateTime.SpecifyKind(midnight - earliest, DateTimeKind.Utc);
        }

        return DateTime.SpecifyKind(TimeZoneInfo.ConvertTimeToUtc(midnight, Zone), DateTimeKind.Utc);
    }

    /// <summary>The UTC instant as a DateTime of Kind Utc (Unspecified is read as UTC, Local is converted).</summary>
    public static DateTime AsUtc(DateTime value) => value.Kind switch
    {
        DateTimeKind.Local => value.ToUniversalTime(),
        DateTimeKind.Unspecified => DateTime.SpecifyKind(value, DateTimeKind.Utc),
        _ => value
    };

    private static TimeZoneInfo? TryResolveZone(string? id)
    {
        var trimmed = id?.Trim();
        if (string.IsNullOrEmpty(trimmed) || trimmed.Length > ActivityConsts.MaxTimeZoneLength)
        {
            return null;
        }

        return TimeZoneInfo.TryFindSystemTimeZoneById(trimmed, out var zone) ? zone : null;
    }

    private TimeZoneInfo? DeviceZone(string? id)
    {
        if (string.IsNullOrWhiteSpace(id))
        {
            return null;
        }

        if (!_deviceZones.TryGetValue(id, out var zone))
        {
            zone = TryResolveZone(id);
            _deviceZones[id] = zone;
        }

        return zone;
    }
}

/// <summary>A wall clock: either a time zone (DST-aware) or a fixed UTC offset.</summary>
public readonly struct LocalClock
{
    private readonly TimeZoneInfo? _zone;
    private readonly TimeSpan _offset;

    private LocalClock(TimeZoneInfo? zone, TimeSpan offset)
    {
        _zone = zone;
        _offset = offset;
    }

    public static LocalClock Of(TimeZoneInfo zone) => new(zone, TimeSpan.Zero);

    public static LocalClock Fixed(TimeSpan utcOffset) => new(null, utcOffset);

    /// <summary>The local wall-clock time (Kind Unspecified) of a UTC instant.</summary>
    public DateTime ToLocal(DateTime utc)
    {
        var instant = ReportCalendar.AsUtc(utc);
        var local = _zone == null ? instant + _offset : TimeZoneInfo.ConvertTimeFromUtc(instant, _zone);
        return DateTime.SpecifyKind(local, DateTimeKind.Unspecified);
    }
}
