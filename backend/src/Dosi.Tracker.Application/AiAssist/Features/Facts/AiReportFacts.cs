using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Read;

namespace Dosi.Tracker.AiAssist.Features.Facts;

/// <summary>The reads one report's explanation is built from (spec 11).</summary>
[Flags]
public enum AiReportSources
{
    None = 0,
    Summary = 1,
    Daily = 2,
    MemberDays = 4,
    Hourly = 8,
    AppUsage = 16,
    Attendance = 32,
    Devices = 64
}

/// <summary>
/// The per-report data plan of report-explanation and the facts only reports use: members per day rolled up,
/// member × project time, hours of the day, attendance and devices (platform and agent version only; device names and
/// OS versions never reach the read models). Members are the writer's handles; rows of members without one are left out
/// (the totals still count them).
/// </summary>
public static class AiReportFacts
{
    public const int MaxMemberProjectRows = 40;
    public const int MaxDeviceRows = 24;

    public const string MemberProjectsList = "memberProjects";
    public const string HoursList = "hours";
    public const string AttendanceList = "attendance";
    public const string DevicesList = "devices";

    public const string AttendanceCaveat =
        "In attendance, absent means no tracked time on a working day; it says nothing about why.";

    private static readonly Regex VersionPrefix = new(@"^\d+(\.\d+){0,3}", RegexOptions.CultureInvariant, TimeSpan.FromSeconds(1));

    /// <summary>What each report's explanation reads (a summary always: the totals and the previous period).</summary>
    public static AiReportSources SourcesFor(string slug) => slug switch
    {
        "time-activity" => AiReportSources.Summary | AiReportSources.Daily,
        "weekly" => AiReportSources.Summary | AiReportSources.Daily | AiReportSources.MemberDays,
        "projects" => AiReportSources.Summary,
        "productivity" => AiReportSources.Summary | AiReportSources.Hourly,
        "apps-urls" => AiReportSources.Summary | AiReportSources.AppUsage,
        "attendance" => AiReportSources.Summary | AiReportSources.Attendance,
        "devices" => AiReportSources.Summary | AiReportSources.Devices,
        "work-report" => AiReportSources.Summary | AiReportSources.Daily,
        "analytics" => AiReportSources.Summary | AiReportSources.Daily | AiReportSources.Hourly,
        _ => throw new ArgumentException($"'{slug}' is not an explainable report.", nameof(slug))
    };

    /// <summary>The list the report's deterministic summary lists its top rows from.</summary>
    public static string MainListFor(string slug) => slug switch
    {
        "projects" or "work-report" or "analytics" => AiPeriodFacts.ProjectsList,
        "apps-urls" => AiPeriodFacts.AppsList,
        "attendance" => AttendanceList,
        "devices" => DevicesList,
        _ => AiPeriodFacts.MembersList
    };

    /// <summary>Per member with a handle: the days of the range with tracked time (the members' names come from the
    /// summary's rows, added before).</summary>
    public static void AddMemberDays(AiBundleWriter writer, IReadOnlyList<AiMemberDayRow> rows)
    {
        var list = new JsonArray();
        foreach (var member in rows.GroupBy(r => r.UserId)
                     .Select(g => (UserId: g.Key, Minutes: g.Sum(r => r.TrackedMinutes), Days: g.Count(r => r.TrackedMinutes > 0)))
                     .OrderByDescending(m => m.Minutes)
                     .ThenBy(m => m.UserId))
        {
            if (!writer.Pseudonymizer.TryGetHandle(member.UserId, out var handle))
            {
                continue;
            }

            list.Add(new JsonObject
            {
                ["member"] = handle,
                ["activeDays"] = writer.AddFact("days with tracked time", AiPeriodFacts.Count(member.Days), handle)
            });
        }

        if (list.Count > 0)
        {
            writer.SetList("memberActiveDays", list);
        }
    }

    /// <summary>Member × project time for listed members and projects (at most <see cref="MaxMemberProjectRows"/>).</summary>
    public static void AddMemberProjects(AiBundleWriter writer, IReadOnlyList<AiMemberProjectRow> rows, IReadOnlyCollection<Guid> listedProjects)
    {
        var list = new JsonArray();
        foreach (var row in rows.Where(r => r.TrackedMinutes > 0).OrderByDescending(r => r.TrackedMinutes).ThenBy(r => r.UserId).ThenBy(r => r.ProjectId))
        {
            if (list.Count >= MaxMemberProjectRows)
            {
                break;
            }

            if (!writer.Pseudonymizer.TryGetHandle(row.UserId, out var handle) || !listedProjects.Contains(row.ProjectId))
            {
                continue;
            }

            var project = writer.AddProject(row.ProjectId, row.ProjectTitle);
            list.Add(new JsonObject
            {
                ["member"] = handle,
                ["project"] = project,
                ["tracked"] = writer.AddFact("tracked time of the member on the project", DisplayFormat.Duration(row.TrackedMinutes), handle)
            });
        }

        if (list.Count > 0)
        {
            writer.SetList(MemberProjectsList, list);
        }
    }

    /// <summary>Tracked time and activity % per local hour of the day (summed over the weekdays), hours with time only.</summary>
    public static void AddHours(AiBundleWriter writer, IReadOnlyList<AiHourlyRow> hourly)
    {
        var list = new JsonArray();
        foreach (var hour in hourly.GroupBy(h => h.Hour).OrderBy(g => g.Key))
        {
            var tracked = hour.Sum(h => h.TrackedMinutes);
            if (tracked <= 0)
            {
                continue;
            }

            var active = hour.Sum(h => h.ActiveMinutes);
            list.Add(new JsonObject
            {
                ["hour"] = writer.AddFact("hour of the day", $"{hour.Key:00}:00"),
                ["tracked"] = writer.AddFact("tracked time in that hour", DisplayFormat.Duration(tracked)),
                ["activity"] = writer.AddFact("activity % in that hour", AiPeriodFacts.Share(active, tracked))
            });
        }

        if (list.Count > 0)
        {
            writer.SetList(HoursList, list);
        }
    }

    /// <summary>Per member: working days present and without tracked time, weekend days worked and worked time.</summary>
    /// <param name="names">Display names by member (the rows carry user names).</param>
    public static void AddAttendance(AiBundleWriter writer, AiAttendance attendance, IReadOnlyDictionary<Guid, string> names)
    {
        var list = new JsonArray();
        foreach (var row in attendance.Rows.OrderByDescending(r => r.WorkedMinutes).ThenBy(r => r.UserId))
        {
            if (!writer.Pseudonymizer.TryGetHandle(row.UserId, out _))
            {
                continue;
            }

            var handle = writer.AddMember(row.UserId, NameOf(names, row.UserId, row.UserName));
            list.Add(new JsonObject
            {
                ["member"] = handle,
                ["present"] = writer.AddFact("working days with tracked time", AiPeriodFacts.Count(row.DaysPresent), handle),
                ["absent"] = writer.AddFact("working days without tracked time", AiPeriodFacts.Count(row.DaysAbsent), handle),
                ["weekendDays"] = writer.AddFact("weekend days with tracked time", AiPeriodFacts.Count(row.WeekendDaysWorked), handle),
                ["tracked"] = writer.AddFact("worked time", DisplayFormat.Duration(row.WorkedMinutes), handle)
            });
        }

        if (list.Count == 0)
        {
            return;
        }

        writer.SetList(AttendanceList, list);
        writer.SetList("workingDays", writer.AddFact("working days in the range", AiPeriodFacts.Count(attendance.Days.Count)));
        writer.AddCaveat(AttendanceCaveat);
    }

    /// <summary>Per member device: platform, agent version, tracked time (at most <see cref="MaxDeviceRows"/>).</summary>
    /// <param name="names">Display names by member (the rows carry user names).</param>
    public static void AddDevices(AiBundleWriter writer, IReadOnlyList<AiDeviceRow> devices, IReadOnlyDictionary<Guid, string> names)
    {
        var list = new JsonArray();
        foreach (var device in devices.OrderByDescending(d => d.TrackedMinutes).ThenBy(d => d.UserId).ThenBy(d => d.DeviceId))
        {
            if (list.Count >= MaxDeviceRows)
            {
                break;
            }

            if (!writer.Pseudonymizer.TryGetHandle(device.UserId, out _))
            {
                continue;
            }

            var handle = writer.AddMember(device.UserId, NameOf(names, device.UserId, device.UserName));
            list.Add(new JsonObject
            {
                ["member"] = handle,
                ["platform"] = writer.AddFact("agent platform", Platform(device.Platform), handle),
                ["agentVersion"] = writer.AddFact("agent version", AgentVersion(device.AgentVersion), handle),
                ["tracked"] = writer.AddFact("tracked time with the device", DisplayFormat.Duration(device.TrackedMinutes), handle)
            });
        }

        if (list.Count > 0)
        {
            writer.SetList(DevicesList, list);
        }
    }

    /// <summary>A platform as shown (the agents' known platforms; anything else is "unknown").</summary>
    public static string Platform(string? platform) =>
        platform?.Trim().ToLowerInvariant() is "windows" or "macos" or "linux" ? platform.Trim().ToLowerInvariant() : "unknown";

    /// <summary>An agent version as shown: its numeric prefix ("1.4.2"), "unknown" when there is none. Agents report
    /// it, so nothing else of it is ever sent.</summary>
    public static string AgentVersion(string? version)
    {
        var match = VersionPrefix.Match(version?.Trim() ?? string.Empty);
        return match.Success ? match.Value : "unknown";
    }

    /// <summary>The numeric prefix of an agent version as a <see cref="Version"/> (a lone major gets ".0"), or null
    /// when it has none.</summary>
    public static Version? ParseVersion(string? version)
    {
        var prefix = AgentVersion(version);
        if (prefix == "unknown")
        {
            return null;
        }

        return Version.TryParse(prefix.Contains('.') ? prefix : prefix + ".0", out var parsed) ? parsed : null;
    }

    private static string NameOf(IReadOnlyDictionary<Guid, string> names, Guid userId, string userName) =>
        names.TryGetValue(userId, out var name) && !string.IsNullOrWhiteSpace(name) ? name : userName;
}
