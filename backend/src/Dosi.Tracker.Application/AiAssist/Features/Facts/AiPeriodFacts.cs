using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json.Nodes;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;

namespace Dosi.Tracker.AiAssist.Features.Facts;

/// <summary>
/// Turns range read models (totals, days, projects, apps, members) into bundle facts and lists, with the caps of spec
/// 11. Lists hold ids only: <c>totals</c> maps names to fact ids, <c>days</c>, <c>projects</c>, <c>apps</c> and
/// <c>members</c> are arrays of id objects, and what does not fit is folded into <c>otherProjects</c>,
/// <c>otherApps</c> and <c>otherMembers</c>. Members get the writer's handles; one without a handle (beyond the card's
/// cap) is folded. Used by the period summary, the report explanation and the assistant's tools.
/// </summary>
public static class AiPeriodFacts
{
    public const string TotalsList = "totals";
    public const string DaysList = "days";
    public const string WeeksList = "weeks";
    public const string MonthsList = "months";
    public const string BusiestDayList = "busiestDay";
    public const string LightestDayList = "lightestDay";
    public const string ProjectsList = "projects";
    public const string OtherProjectsList = "otherProjects";
    public const string AppsList = "apps";
    public const string OtherAppsList = "otherApps";
    public const string MembersList = "members";
    public const string OtherMembersList = "otherMembers";

    // Keys of the totals list.
    public const string Tracked = "tracked";
    public const string Idle = "idle";
    public const string Activity = "activity";
    public const string Blocks = "blocks";
    public const string Period = "period";
    public const string TimeZone = "timeZone";
    public const string PreviousTracked = "previousTracked";
    public const string Change = "change";
    public const string ChangePercent = "changePercent";
    public const string UnmeasuredShare = "unmeasuredShare";

    public const string UnmeasuredCaveat =
        "Activity % was not measured for part of the tracked time; that part is not counted as zero.";

    public const string IncludesTodayCaveat = "The period includes today, so its figures may still change.";

    public const string ArchivedProjectsCaveat = "Some of these projects are archived.";

    private static readonly CultureInfo Culture = CultureInfo.InvariantCulture;

    /// <summary>A count as it is shown ("12").</summary>
    public static string Count(long value) => value.ToString(Culture);

    /// <summary>Tracked time first (so it is <c>F1</c>), then idle time, activity % and blocks.</summary>
    /// <param name="subject">Whom the totals are about (a member handle or a project id), for the reader only.</param>
    public static void AddTotals(AiBundleWriter writer, JsonObject totals, AiSummary summary, string? subject = null)
    {
        totals[Tracked] = writer.AddFact("tracked time", DisplayFormat.Duration(summary.TrackedMinutes), subject);
        totals[Idle] = writer.AddFact("idle time", DisplayFormat.Duration(summary.IdleMinutes), subject);
        totals[Activity] = writer.AddFact("activity % (share of minutes with keyboard or mouse input)",
            DisplayFormat.Percent(summary.AverageProductivity), subject);
        totals[Blocks] = writer.AddFact("tracked blocks", Count(summary.Blocks), subject);
    }

    /// <summary>The period ("Mon 20 Oct to Sun 26 Oct") and its time zone; the caveat when it includes today.</summary>
    public static void AddRange(AiBundleWriter writer, JsonObject totals, DateTime fromUtc, DateTime toUtc, TimeZoneInfo zone, DateTime nowUtc)
    {
        var (first, last) = LocalDays(fromUtc, toUtc, zone);
        var withYear = DisplayFormat.NeedsYear(first, last, Today(zone, nowUtc));
        var period = first == last
            ? DisplayFormat.Day(first, withYear)
            : DisplayFormat.Day(first, withYear) + " to " + DisplayFormat.Day(last, withYear);

        totals[Period] = writer.AddFact("period", period);
        totals[TimeZone] = writer.AddFact("time zone", DisplayFormat.Zone(zone, fromUtc));

        if (IncludesNow(fromUtc, toUtc, nowUtc))
        {
            writer.AddCaveat(IncludesTodayCaveat);
        }
    }

    /// <summary>The previous period of the same length: its tracked time and the change.</summary>
    public static void AddComparison(AiBundleWriter writer, JsonObject totals, AiSummary current, AiSummary previous, string? subject = null)
    {
        totals[PreviousTracked] = writer.AddFact("tracked time in the previous period of the same length",
            DisplayFormat.Duration(previous.TrackedMinutes), subject);
        totals[Change] = writer.AddFact("change in tracked time from the previous period",
            DisplayFormat.Delta(current.TrackedMinutes - previous.TrackedMinutes), subject);

        if (DisplayFormat.DeltaPercent(current.TrackedMinutes, previous.TrackedMinutes) is { } percent)
        {
            totals[ChangePercent] = writer.AddFact("relative change in tracked time from the previous period", percent, subject);
        }
    }

    /// <summary>The share of tracked time whose activity % was not measured, with its caveat (nothing when all was
    /// measured).</summary>
    public static void AddUnmeasured(AiBundleWriter writer, JsonObject totals, IReadOnlyList<AiDataQualityRow> rows)
    {
        var tracked = rows.Sum(r => r.TrackedMinutes);
        var unmeasured = rows.Sum(r => r.UnmeasuredMinutes);
        if (tracked <= 0 || unmeasured <= 0)
        {
            return;
        }

        totals[UnmeasuredShare] = writer.AddFact("share of tracked time without activity measurement",
            DisplayFormat.Percent(Math.Min(100, unmeasured / tracked * 100)));
        writer.AddCaveat(UnmeasuredCaveat);
    }

    /// <summary>One point per local day (at most <paramref name="max"/>), plus the busiest and the lightest day with
    /// tracked time (when two or more days have some).</summary>
    public static void AddDays(AiBundleWriter writer, IReadOnlyList<AiDailyRow> days, bool withYear, int max = AiAssistConsts.MaxBundleDailyPoints)
    {
        var points = new JsonArray();
        var ids = new List<(AiDailyRow Day, string Date, string Tracked)>();
        foreach (var day in days.OrderBy(d => d.Date).Take(Math.Max(0, max)))
        {
            var date = writer.AddFact("day", DisplayFormat.Day(day.Date, withYear));
            var tracked = writer.AddFact("tracked time that day", DisplayFormat.Duration(day.TrackedMinutes));
            ids.Add((day, date, tracked));
            points.Add(new JsonObject { ["date"] = date, ["tracked"] = tracked });
        }

        if (points.Count == 0)
        {
            return;
        }

        writer.SetList(DaysList, points);
        var active = ids.Where(d => d.Day.TrackedMinutes > 0).ToList();
        if (active.Count >= 2)
        {
            var busiest = active.OrderByDescending(d => d.Day.TrackedMinutes).ThenBy(d => d.Day.Date).First();
            var lightest = active.OrderBy(d => d.Day.TrackedMinutes).ThenBy(d => d.Day.Date).First();
            writer.SetList(BusiestDayList, new JsonObject { ["date"] = busiest.Date, ["tracked"] = busiest.Tracked });
            writer.SetList(LightestDayList, new JsonObject { ["date"] = lightest.Date, ["tracked"] = lightest.Tracked });
        }
    }

    /// <summary>
    /// Points for a report's range: days up to <see cref="AiAssistConsts.MaxBundleDailyPoints"/> days, weeks (Monday to
    /// Sunday) up to <see cref="AiAssistConsts.MaxAssistantRangeDays"/> days, months beyond.
    /// </summary>
    public static void AddPoints(AiBundleWriter writer, IReadOnlyList<AiDailyRow> days, int rangeDays, bool withYear)
    {
        if (rangeDays <= AiAssistConsts.MaxBundleDailyPoints)
        {
            AddDays(writer, days, withYear);
            return;
        }

        var weekly = rangeDays <= AiAssistConsts.MaxAssistantRangeDays;
        var buckets = days
            .GroupBy(d => weekly ? MondayOf(d.Date) : new DateTime(d.Date.Year, d.Date.Month, 1))
            .OrderBy(g => g.Key)
            .Take(AiAssistConsts.MaxBundleDailyPoints);

        var points = new JsonArray();
        foreach (var bucket in buckets)
        {
            var label = weekly ? "week starting" : "month";
            var display = weekly ? DisplayFormat.Day(bucket.Key, withYear) : bucket.Key.ToString("MMM yyyy", Culture);
            var start = writer.AddFact(label, display);
            var tracked = writer.AddFact(weekly ? "tracked time that week" : "tracked time that month",
                DisplayFormat.Duration(bucket.Sum(d => d.TrackedMinutes)));
            points.Add(new JsonObject { [weekly ? "week" : "month"] = start, ["tracked"] = tracked });
        }

        if (points.Count > 0)
        {
            writer.SetList(weekly ? WeeksList : MonthsList, points);
        }
    }

    /// <summary>The <paramref name="max"/> projects with the most tracked time, with their share of
    /// <paramref name="totalMinutes"/>; the rest folded. Adds the caveat when a listed project is archived.</summary>
    public static void AddProjects(AiBundleWriter writer, IReadOnlyList<AiProjectRow> projects, double totalMinutes, int max)
    {
        var ranked = projects.Where(p => p.TrackedMinutes > 0)
            .OrderByDescending(p => p.TrackedMinutes)
            .ThenBy(p => p.ProjectId)
            .ToList();
        if (ranked.Count == 0)
        {
            return;
        }

        var list = new JsonArray();
        foreach (var project in ranked.Take(max))
        {
            var id = writer.AddProject(project.ProjectId, project.Title);
            list.Add(new JsonObject
            {
                ["project"] = id,
                ["tracked"] = writer.AddFact("tracked time on the project", DisplayFormat.Duration(project.TrackedMinutes), id),
                ["share"] = writer.AddFact("share of the tracked time", Share(project.TrackedMinutes, totalMinutes), id)
            });

            if (project.IsArchived)
            {
                writer.AddCaveat(ArchivedProjectsCaveat);
            }
        }

        writer.SetList(ProjectsList, list);
        AddFold(writer, OtherProjectsList, "projects", ranked.Skip(max).Select(p => p.TrackedMinutes).ToList());
    }

    /// <summary>The <paramref name="max"/> apps with the most focused time, with their share; the rest folded.</summary>
    public static void AddApps(AiBundleWriter writer, IReadOnlyList<AiAppUsageRow> apps, int max)
    {
        var ranked = apps.Where(a => a.TrackedMinutes > 0)
            .OrderByDescending(a => a.TrackedMinutes)
            .ThenBy(a => a.App, StringComparer.Ordinal)
            .ToList();
        if (ranked.Count == 0)
        {
            return;
        }

        var total = ranked.Sum(a => a.TrackedMinutes);
        var list = new JsonArray();
        foreach (var app in ranked.Take(max))
        {
            var id = writer.AddApp(app.App);
            list.Add(new JsonObject
            {
                ["app"] = id,
                ["tracked"] = writer.AddFact("time with the app in focus", DisplayFormat.Duration(app.TrackedMinutes), id),
                ["share"] = writer.AddFact("share of the time with an app in focus", Share(app.TrackedMinutes, total), id)
            });
        }

        writer.SetList(AppsList, list);
        AddFold(writer, OtherAppsList, "apps", ranked.Skip(max).Select(a => a.TrackedMinutes).ToList());
    }

    /// <summary>
    /// Members with a handle (the card's cap, see <see cref="AiPseudonymizer.ForCard"/>): their tracked time only, or
    /// with <paramref name="withActivity"/> also their activity %. Members without a handle are folded into
    /// <c>otherMembers</c>.
    /// </summary>
    public static void AddMembers(AiBundleWriter writer, IReadOnlyList<AiMemberRow> members, bool withActivity = false)
    {
        var list = new JsonArray();
        var folded = new List<double>();
        foreach (var member in members.OrderByDescending(m => m.TrackedMinutes).ThenBy(m => m.UserId))
        {
            if (!writer.Pseudonymizer.TryGetHandle(member.UserId, out _))
            {
                folded.Add(member.TrackedMinutes);
                continue;
            }

            var handle = writer.AddMember(member.UserId, member.DisplayName);
            var item = new JsonObject
            {
                ["member"] = handle,
                ["tracked"] = writer.AddFact("tracked time of the member", DisplayFormat.Duration(member.TrackedMinutes), handle)
            };
            if (withActivity)
            {
                item["activity"] = writer.AddFact("activity % of the member", DisplayFormat.Percent(member.AverageProductivity), handle);
            }

            list.Add(item);
        }

        if (list.Count > 0)
        {
            writer.SetList(MembersList, list);
        }

        AddFold(writer, OtherMembersList, "members", folded);
    }

    /// <summary>The handles for a card's members, by tracked minutes (see <see cref="AiPseudonymizer.ForCard"/>).</summary>
    public static AiPseudonymizer Pseudonymizer(Guid callerId, IEnumerable<AiMemberRow> members, int max = AiAssistConsts.MaxBundleMembers) =>
        AiPseudonymizer.ForCard(callerId, members.Select(m => new AiMemberWeight(m.UserId, m.TrackedMinutes)), max);

    /// <summary>The first and last local day of [fromUtc, toUtc).</summary>
    public static (DateTime First, DateTime Last) LocalDays(DateTime fromUtc, DateTime toUtc, TimeZoneInfo zone)
    {
        var clock = LocalClock.Of(zone);
        var first = clock.ToLocal(fromUtc).Date;
        var last = clock.ToLocal(ReportCalendar.AsUtc(toUtc).AddTicks(-1)).Date;
        return (first, last < first ? first : last);
    }

    /// <summary>Today on the zone's clock.</summary>
    public static DateTime Today(TimeZoneInfo zone, DateTime nowUtc) => LocalClock.Of(zone).ToLocal(nowUtc).Date;

    /// <summary>True when [fromUtc, toUtc) holds <paramref name="nowUtc"/>.</summary>
    public static bool IncludesNow(DateTime fromUtc, DateTime toUtc, DateTime nowUtc)
    {
        var now = ReportCalendar.AsUtc(nowUtc);
        return ReportCalendar.AsUtc(fromUtc) <= now && now < ReportCalendar.AsUtc(toUtc);
    }

    /// <summary>The number of whole days in [fromUtc, toUtc), rounded up.</summary>
    public static int RangeDays(DateTime fromUtc, DateTime toUtc) =>
        (int)Math.Ceiling((ReportCalendar.AsUtc(toUtc) - ReportCalendar.AsUtc(fromUtc)).TotalDays);

    /// <summary>"35%" of a whole; "0%" when the whole is empty.</summary>
    public static string Share(double part, double whole) =>
        DisplayFormat.Percent(whole <= 0 ? 0 : Math.Min(100, part / whole * 100));

    private static void AddFold(AiBundleWriter writer, string list, string what, IReadOnlyCollection<double> minutes)
    {
        if (minutes.Count == 0)
        {
            return;
        }

        writer.SetList(list, new JsonObject
        {
            ["count"] = writer.AddFact($"other {what} (not listed)", Count(minutes.Count)),
            ["tracked"] = writer.AddFact($"tracked time of the other {what}", DisplayFormat.Duration(minutes.Sum()))
        });
    }

    private static DateTime MondayOf(DateTime day) => day.Date.AddDays(-(((int)day.DayOfWeek + 6) % 7));
}
