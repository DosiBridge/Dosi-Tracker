using System;
using System.Collections.Generic;
using System.Linq;

namespace Dosi.Tracker.AiAssist.Read;

// What AI features and tools may read, as the caller (IAiReadLayer). These are projections of the product's own DTOs
// that drop, at the source, everything AI assist never uses: emails, device names, OS versions, keyboard and mouse
// counts, per-minute timelines, captures and pay. Names stay (server-side only) so answers can be rendered for the
// reader; the bundle writer replaces them with handles before anything is sent.

/// <summary>A reporting question: a UTC range [From, To) and optional filters.</summary>
/// <param name="UserId">One member; ignored (the caller is used) for callers who only see themselves, and refused
/// for another member unless the caller sees the team.</param>
/// <param name="TimeZone">The zone days are read in; the caller's when null.</param>
/// <param name="DayBasis">"zone" (default) or "member".</param>
public sealed record AiQuery(
    DateTime From,
    DateTime To,
    Guid? ProjectId = null,
    Guid? UserId = null,
    string? TimeZone = null,
    string? DayBasis = null);

/// <summary>One member's totals over the range. <paramref name="DisplayName"/> is the full name, else the user name.</summary>
public sealed record AiMemberRow(
    Guid UserId,
    string DisplayName,
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    double? AverageProductivity);

public sealed record AiProjectRow(
    Guid ProjectId,
    string Title,
    bool IsArchived,
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    double? AverageProductivity);

/// <summary>One member's time on one project (no pay).</summary>
public sealed record AiMemberProjectRow(
    Guid UserId,
    string DisplayName,
    Guid ProjectId,
    string ProjectTitle,
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    double? AverageProductivity);

/// <summary>Totals over the range (no keyboard or mouse totals).</summary>
public sealed record AiSummary(
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    double? AverageProductivity,
    IReadOnlyList<AiMemberRow> Members,
    IReadOnlyList<AiProjectRow> Projects,
    IReadOnlyList<AiMemberProjectRow> MemberProjects);

/// <summary>One local day (no keyboard or mouse totals).</summary>
/// <param name="Date">The local date at 00:00.</param>
public sealed record AiDailyRow(
    DateTime Date,
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    double? AverageProductivity);

/// <summary>One member's work on one local day.</summary>
public sealed record AiMemberDayRow(
    Guid UserId,
    string UserName,
    DateTime Date,
    int Blocks,
    double TrackedMinutes,
    double IdleMinutes,
    DateTime FirstStartedAt,
    DateTime LastEndedAt);

/// <summary>One app's focused time (its canonical name).</summary>
public sealed record AiAppUsageRow(
    string App,
    double TrackedMinutes,
    double IdleMinutes,
    int Blocks,
    int Members);

public sealed record AiAttendanceRow(
    Guid UserId,
    string UserName,
    int DaysPresent,
    int DaysAbsent,
    int WeekendDaysWorked,
    double WorkedMinutes,
    double IdleMinutes,
    double? AverageStartMinutes,
    double? AverageEndMinutes);

/// <param name="Days">The weekdays of the range (local dates at 00:00).</param>
public sealed record AiAttendance(IReadOnlyList<DateTime> Days, IReadOnlyList<AiAttendanceRow> Rows);

/// <param name="DayOfWeek">0 = Sunday … 6 = Saturday.</param>
public sealed record AiHourlyRow(int DayOfWeek, int Hour, double TrackedMinutes, double ActiveMinutes);

/// <summary>One device a member tracked from: its platform and agent version only (no device name, no OS version).</summary>
public sealed record AiDeviceRow(
    Guid UserId,
    string UserName,
    Guid? DeviceId,
    string? Platform,
    string? AgentVersion,
    DateTime LastActivityAt,
    double TrackedMinutes);

public sealed record AiDataQualityRow(
    Guid UserId,
    string UserName,
    int Blocks,
    double TrackedMinutes,
    double UnmeasuredMinutes,
    int? MaxAbsClockSkewSeconds,
    int BlocksWithInjectedInput,
    DateTime? LastActivityAt);

/// <summary>Focused seconds of one app in a block (canonical name; seconds null from older agents).</summary>
public sealed record AiAppSeconds(string App, int? Seconds);

/// <summary>One window title of a block, read only when the workspace allows window titles.</summary>
public sealed record AiWindowTitle(string App, string Title, int? Seconds);

/// <summary>
/// One tracked block: no input counts, no timeline, no device, no captures. <paramref name="WindowTitles"/> is null
/// unless the workspace allows window titles, <paramref name="Note"/> null unless it allows notes.
/// </summary>
public sealed record AiBlock(
    Guid Id,
    Guid UserId,
    Guid ProjectId,
    DateTime StartedAt,
    DateTime EndedAt,
    int? Productivity,
    int? IdleSeconds,
    string? EndReason,
    int? ClockSkewSeconds,
    int? InjectedInputEvents,
    IReadOnlyList<AiAppSeconds> Apps,
    IReadOnlyList<AiWindowTitle>? WindowTitles,
    string? Note)
{
    public double Minutes => Math.Max(0, (EndedAt - StartedAt).TotalMinutes);
}

/// <summary>A member as the caller may see them (no email). The names are for matching and rendering only.</summary>
public sealed record AiRosterEntry(
    Guid UserId,
    string UserName,
    string? Name,
    string? Surname,
    string Role,
    bool IsClient,
    DateTime JoinedAt)
{
    /// <summary>The full name, else the user name.</summary>
    public string DisplayName
    {
        get
        {
            var full = string.Join(" ", new[] { Name, Surname }.Where(p => !string.IsNullOrWhiteSpace(p)).Select(p => p!.Trim()));
            return full.Length > 0 ? full : UserName;
        }
    }
}

public sealed record AiProject(Guid Id, string Title, string? Description, bool IsArchived);

/// <summary>A task without its description.</summary>
public sealed record AiTask(Guid Id, string Title, bool IsCompleted, DateTime? CompletedAt);

/// <param name="TotalCount">All matching tasks, also those beyond <paramref name="Items"/>.</param>
public sealed record AiTaskPage(long TotalCount, IReadOnlyList<AiTask> Items);
