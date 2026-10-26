using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Globalization;
using Dosi.Tracker.Reporting;
using Volo.Abp;
using Volo.Abp.Validation;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>A resolved period: the UTC range [<paramref name="FromUtc"/>, <paramref name="ToUtc"/>) of whole local days
/// <paramref name="FirstDay"/> to <paramref name="LastDay"/> (inclusive, at 00:00) in <paramref name="TimeZone"/>.</summary>
public sealed record AiPeriod(DateTime FromUtc, DateTime ToUtc, DateTime FirstDay, DateTime LastDay, string TimeZone)
{
    /// <summary>The number of local days.</summary>
    public int Days => (int)(LastDay - FirstDay).TotalDays + 1;
}

/// <summary>
/// Turns a named period into a UTC range of whole local days in the caller's zone (spec 10.1). Days start at
/// <see cref="ReportCalendar.StartOfDayUtc"/>, so DST changes and half-hour or 45-minute zones are exact. Weeks run
/// Monday to Sunday. Periods that include today end with today (nothing is reported for days that have not happened):
/// <list type="bullet">
/// <item><c>today</c>, <c>yesterday</c>;</item>
/// <item><c>this_week</c> (Monday to today), <c>last_week</c> (the previous Monday to Sunday);</item>
/// <item><c>last_7_days</c> and <c>last_30_days</c> (ending today);</item>
/// <item><c>this_month</c> (the 1st to today), <c>last_month</c>;</item>
/// <item><c>custom</c>: inclusive local dates (yyyy-MM-dd), at most <see cref="AiAssistConsts.MaxAssistantRangeDays"/>
/// days.</item>
/// </list>
/// </summary>
public static class AiPeriodResolver
{
    public const string Today = "today";
    public const string Yesterday = "yesterday";
    public const string ThisWeek = "this_week";
    public const string LastWeek = "last_week";
    public const string Last7Days = "last_7_days";
    public const string ThisMonth = "this_month";
    public const string LastMonth = "last_month";
    public const string Last30Days = "last_30_days";
    public const string Custom = "custom";

    public static readonly IReadOnlyList<string> Periods = new[]
    {
        Today, Yesterday, ThisWeek, LastWeek, Last7Days, ThisMonth, LastMonth, Last30Days, Custom
    };

    public const string DateFormat = "yyyy-MM-dd";

    /// <param name="period">One of <see cref="Periods"/>.</param>
    /// <param name="timeZone">An IANA zone; UTC when blank or unknown.</param>
    /// <param name="nowUtc">The current instant.</param>
    /// <param name="from">For <see cref="Custom"/>: the first local date (yyyy-MM-dd).</param>
    /// <param name="to">For <see cref="Custom"/>: the last local date, inclusive.</param>
    /// <exception cref="AbpValidationException">An unknown period, or custom dates that are missing or malformed.</exception>
    /// <exception cref="BusinessException"><c>Tracker:ReportRangeInvalid</c> when the last date is before the first;
    /// <c>Tracker:AiRangeTooLarge</c> when a custom range is longer than the cap.</exception>
    public static AiPeriod Resolve(string? period, string? timeZone, DateTime nowUtc, string? from = null, string? to = null)
    {
        var calendar = new ReportCalendar(ReportCalendar.ResolveZone(timeZone), memberBasis: false);
        var today = calendar.ZoneClock.ToLocal(ReportCalendar.AsUtc(nowUtc)).Date;

        var (first, last) = (period?.Trim().ToLowerInvariant()) switch
        {
            Today => (today, today),
            Yesterday => (today.AddDays(-1), today.AddDays(-1)),
            ThisWeek => (MondayOf(today), today),
            LastWeek => (MondayOf(today).AddDays(-7), MondayOf(today).AddDays(-1)),
            Last7Days => (today.AddDays(-6), today),
            ThisMonth => (new DateTime(today.Year, today.Month, 1), today),
            LastMonth => (new DateTime(today.Year, today.Month, 1).AddMonths(-1), new DateTime(today.Year, today.Month, 1).AddDays(-1)),
            Last30Days => (today.AddDays(-29), today),
            Custom => CustomRange(from, to),
            _ => throw Invalid("period", "Choose one of: " + string.Join(", ", Periods) + ".")
        };

        if (last < first)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid);
        }

        if ((last - first).TotalDays + 1 > AiAssistConsts.MaxAssistantRangeDays)
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiRangeTooLarge)
                .WithData("maxDays", AiAssistConsts.MaxAssistantRangeDays);
        }

        return new AiPeriod(
            calendar.StartOfDayUtc(first),
            calendar.StartOfDayUtc(last.AddDays(1)),
            DateTime.SpecifyKind(first, DateTimeKind.Unspecified),
            DateTime.SpecifyKind(last, DateTimeKind.Unspecified),
            calendar.Zone.Id);
    }

    /// <summary>The UTC range of one local day (yyyy-MM-dd) in <paramref name="timeZone"/>.</summary>
    /// <exception cref="AbpValidationException">The date is malformed.</exception>
    public static AiPeriod ResolveDay(string? day, string? timeZone)
    {
        var date = ParseDate(day, "date");
        var calendar = new ReportCalendar(ReportCalendar.ResolveZone(timeZone), memberBasis: false);
        return new AiPeriod(
            calendar.StartOfDayUtc(date),
            calendar.StartOfDayUtc(date.AddDays(1)),
            date,
            date,
            calendar.Zone.Id);
    }

    private static (DateTime First, DateTime Last) CustomRange(string? from, string? to) =>
        (ParseDate(from, "from"), ParseDate(to, "to"));

    private static DateTime ParseDate(string? value, string member)
    {
        if (!DateTime.TryParseExact(value?.Trim(), DateFormat, CultureInfo.InvariantCulture, DateTimeStyles.None, out var date) ||
            date.Year is < 1970 or > 9000)
        {
            throw Invalid(member, $"Use a date in the form {DateFormat}.");
        }

        return DateTime.SpecifyKind(date.Date, DateTimeKind.Unspecified);
    }

    private static DateTime MondayOf(DateTime day) => day.AddDays(-(((int)day.DayOfWeek + 6) % 7));

    private static AbpValidationException Invalid(string member, string message) =>
        new("The period is not valid.", new List<ValidationResult> { new(message, new[] { member }) });
}
