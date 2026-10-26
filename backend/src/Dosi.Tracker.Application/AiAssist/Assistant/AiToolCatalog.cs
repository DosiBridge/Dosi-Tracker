using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;
using Microsoft.Extensions.AI;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Timing;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Builds the AIFunctions one Ask AI turn offers the model (spec 10.2): 11 tools in a fixed order, bound to this
/// turn's <see cref="AiTurnContext"/> and read as the caller through <see cref="IAiReadLayer"/>. A member (who may
/// not see the team) gets every tool but <c>list_team</c>.
/// </summary>
public class AiToolCatalog : ITransientDependency
{
    private readonly IAiReadLayer _read;
    private readonly AiToolGate _gate;
    private readonly AiHelpLibrary _help;
    private readonly IClock _clock;

    public AiToolCatalog(IAiReadLayer read, AiToolGate gate, AiHelpLibrary help, IClock clock)
    {
        _read = read;
        _gate = gate;
        _help = help;
        _clock = clock;
    }

    public virtual IReadOnlyList<AITool> For(AiCallerContext caller, AiTurnContext turn)
    {
        var tools = new List<AITool>
        {
            new GetTimeSummaryTool(turn, _gate, _read, _clock),
            new ComparePeriodsTool(turn, _gate, _read, _clock),
            new GetDailyTrendTool(turn, _gate, _read, _clock),
            new GetMemberDaysTool(turn, _gate, _read, _clock),
            new GetAttendanceTool(turn, _gate, _read, _clock),
            new GetAppUsageTool(turn, _gate, _read, _clock),
            new GetMemberDayTool(turn, _gate, _read, _clock),
            new ListProjectsTool(turn, _gate, _read)
        };

        tools.Add(new GetProjectTasksTool(turn, _gate, _read));

        if (caller.CanSeeTeam)
        {
            tools.Add(new ListTeamTool(turn, _gate, _read));
        }

        tools.Add(new GetHelpTool(turn, _gate, _help));
        return tools;
    }
}

/// <summary>Arguments and rendering every reporting tool shares.</summary>
internal static class AiToolSupport
{
    public static AiPeriod ResolvePeriod(AIFunctionArguments arguments, string timeZone, DateTime nowUtc, string defaultPeriod = "last_7_days")
    {
        var period = AiTool.ReadStringOrDefault(arguments, "period", defaultPeriod);
        var from = AiTool.ReadStringOrDefault(arguments, "from", null);
        var to = AiTool.ReadStringOrDefault(arguments, "to", null);
        return AiPeriodResolver.Resolve(period, timeZone, nowUtc, from, to);
    }

    public static string PeriodFact(AiTurnContext turn, AiPeriod period, string resultId)
    {
        var today = DateTime.SpecifyKind(period.LastDay, DateTimeKind.Unspecified);
        var needsYear = DisplayFormat.NeedsYear(period.FirstDay, period.LastDay, today);
        var display = period.Days == 1
            ? DisplayFormat.Day(period.FirstDay, needsYear)
            : $"{DisplayFormat.Day(period.FirstDay, needsYear)} - {DisplayFormat.Day(period.LastDay, needsYear)}";
        return turn.Facts.AddFact("the period", display, resultId);
    }

    public static JsonObject Totals(AiTurnContext turn, AiSummary summary, string resultId) => new()
    {
        ["tracked"] = turn.Facts.AddFact("tracked time", DisplayFormat.Duration(summary.TrackedMinutes), resultId),
        ["idle"] = turn.Facts.AddFact("idle time", DisplayFormat.Duration(summary.IdleMinutes), resultId),
        ["activity_percent"] = turn.Facts.AddFact("activity percent", DisplayFormat.Percent(summary.AverageProductivity), resultId),
        ["blocks"] = turn.Facts.AddFact("number of tracked blocks", summary.Blocks.ToString(System.Globalization.CultureInfo.InvariantCulture), resultId)
    };

    public static JsonArray Members(AiTurnContext turn, IEnumerable<AiMemberRow> members, string resultId, int max)
    {
        var array = new JsonArray();
        foreach (var member in members.OrderByDescending(m => m.TrackedMinutes).Take(max))
        {
            array.Add(new JsonObject
            {
                ["member"] = turn.AddMemberRef(member.UserId, member.DisplayName),
                ["tracked"] = turn.Facts.AddFact("tracked time", DisplayFormat.Duration(member.TrackedMinutes), resultId)
            });
        }

        return array;
    }

    public static JsonArray Projects(AiTurnContext turn, IEnumerable<AiProjectRow> projects, string resultId, int max)
    {
        var array = new JsonArray();
        foreach (var project in projects.OrderByDescending(p => p.TrackedMinutes).Take(max))
        {
            array.Add(new JsonObject
            {
                ["project"] = turn.AddProjectRef(project.ProjectId, project.Title),
                ["title_text"] = Sanitize(project.Title),
                ["tracked"] = turn.Facts.AddFact("tracked time", DisplayFormat.Duration(project.TrackedMinutes), resultId)
            });
        }

        return array;
    }

    public static string Sanitize(string? text) => UntrustedText.Sanitize(text, AiAssistConsts.MaxLabelLength);

    public static AiQuery Query(AiPeriod period, Guid? projectId, Guid? userId, string timeZone) =>
        new(period.FromUtc, period.ToUtc, projectId, userId, timeZone);
}

/// <summary>get_time_summary: totals for the period, scoped to the team or to the caller.</summary>
internal sealed class GetTimeSummaryTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetTimeSummaryTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_time_summary";

    protected override string ToolDescription =>
        "Totals (tracked time, idle time, activity percent, blocks) for a period, for the team or one member.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true), ("project", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "time summary", null, null, async resultId =>
        {
            var (projectId, projectError) = AiToolGate.ResolveProject(Turn, ReadString(arguments, "project"));
            if (projectError != null)
            {
                return projectError;
            }

            var memberArg = ReadString(arguments, "member");
            var (userId, memberError) = AiToolGate.ResolveMember(Turn, memberArg);
            if (memberError != null)
            {
                return memberError;
            }

            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now);
            var query = AiToolSupport.Query(period, projectId, userId, Turn.Caller.TimeZone);
            var summary = await _read.SummaryAsync(Turn.Caller, query);

            var result = new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, userId),
                ["period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["totals"] = AiToolSupport.Totals(Turn, summary, resultId)
            };

            if (Turn.Caller.CanSeeTeam && userId == null)
            {
                result["members"] = AiToolSupport.Members(Turn, summary.Members, resultId, 25);
            }

            result["projects"] = AiToolSupport.Projects(Turn, summary.Projects, resultId, 20);
            return result;
        });
    }
}

/// <summary>compare_periods: the current period against the previous one of the same length, with server-computed
/// deltas.</summary>
internal sealed class ComparePeriodsTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public ComparePeriodsTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "compare_periods";

    protected override string ToolDescription => "The current period against the previous one of the same length, with the change already computed.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true), ("project", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "period comparison", null, null, async resultId =>
        {
            var (projectId, projectError) = AiToolGate.ResolveProject(Turn, ReadString(arguments, "project"));
            if (projectError != null)
            {
                return projectError;
            }

            var (userId, memberError) = AiToolGate.ResolveMember(Turn, ReadString(arguments, "member"));
            if (memberError != null)
            {
                return memberError;
            }

            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now);
            var length = period.ToUtc - period.FromUtc;
            var previous = new AiPeriod(
                period.FromUtc - length, period.FromUtc, period.FirstDay.AddDays(-period.Days), period.FirstDay.AddDays(-1), period.TimeZone);

            var current = await _read.SummaryAsync(Turn.Caller, AiToolSupport.Query(period, projectId, userId, Turn.Caller.TimeZone));
            var before = await _read.SummaryAsync(Turn.Caller, AiToolSupport.Query(previous, projectId, userId, Turn.Caller.TimeZone));

            var result = new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, userId),
                ["current_period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["previous_period"] = AiToolSupport.PeriodFact(Turn, previous, resultId),
                ["current"] = AiToolSupport.Totals(Turn, current, resultId),
                ["previous"] = AiToolSupport.Totals(Turn, before, resultId),
                ["tracked_change"] = Turn.Facts.AddFact(
                    "change in tracked time", DisplayFormat.Delta(current.TrackedMinutes - before.TrackedMinutes), resultId)
            };

            var movers = current.Members
                .Select(m => (m, previousMinutes: before.Members.FirstOrDefault(p => p.UserId == m.UserId)?.TrackedMinutes ?? 0))
                .Select(t => (t.m, change: t.m.TrackedMinutes - t.previousMinutes))
                .OrderByDescending(t => Math.Abs(t.change))
                .Take(5)
                .Select(t => new JsonObject
                {
                    ["member"] = Turn.AddMemberRef(t.m.UserId, t.m.DisplayName),
                    ["change"] = Turn.Facts.AddFact("change in tracked time", DisplayFormat.Delta(t.change), resultId)
                });

            if (Turn.Caller.CanSeeTeam && userId == null)
            {
                result["movers"] = new JsonArray(movers.Cast<JsonNode>().ToArray());
            }

            return result;
        });
    }
}

/// <summary>get_daily_trend: one row per local day of the range.</summary>
internal sealed class GetDailyTrendTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetDailyTrendTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_daily_trend";

    protected override string ToolDescription => "Tracked time and activity percent per day, for the period.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true), ("project", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "daily trend", null, null, async resultId =>
        {
            var (projectId, projectError) = AiToolGate.ResolveProject(Turn, ReadString(arguments, "project"));
            if (projectError != null)
            {
                return projectError;
            }

            var (userId, memberError) = AiToolGate.ResolveMember(Turn, ReadString(arguments, "member"));
            if (memberError != null)
            {
                return memberError;
            }

            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now, "last_30_days");
            var days = await _read.DailyAsync(Turn.Caller, AiToolSupport.Query(period, projectId, userId, Turn.Caller.TimeZone));
            var needsYear = DisplayFormat.NeedsYear(period.FirstDay, period.LastDay, DateTime.SpecifyKind(period.LastDay, DateTimeKind.Unspecified));

            var rows = days.OrderBy(d => d.Date).Take(31).Select(d => new JsonObject
            {
                ["date"] = Turn.Facts.AddFact("date", DisplayFormat.Day(d.Date, needsYear), resultId),
                ["tracked"] = Turn.Facts.AddFact("tracked time", DisplayFormat.Duration(d.TrackedMinutes), resultId),
                ["activity_percent"] = Turn.Facts.AddFact("activity percent", DisplayFormat.Percent(d.AverageProductivity), resultId)
            });

            return new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, userId),
                ["period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["days"] = new JsonArray(rows.Cast<JsonNode>().ToArray())
            };
        });
    }
}

/// <summary>get_member_days: member-days with tracked time, team or the caller's own.</summary>
internal sealed class GetMemberDaysTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetMemberDaysTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_member_days";

    protected override string ToolDescription => "Tracked time per member per day, with first and last activity time.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "member days", null, null, async resultId =>
        {
            var (userId, memberError) = AiToolGate.ResolveMember(Turn, ReadString(arguments, "member"));
            if (memberError != null)
            {
                return memberError;
            }

            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now, "last_7_days");
            var rows = await _read.UserDailyAsync(Turn.Caller, AiToolSupport.Query(period, null, userId, Turn.Caller.TimeZone));
            var zone = ReportCalendar.ResolveZone(Turn.Caller.TimeZone);
            var needsYear = DisplayFormat.NeedsYear(period.FirstDay, period.LastDay, DateTime.SpecifyKind(period.LastDay, DateTimeKind.Unspecified));

            var items = rows.OrderByDescending(r => r.Date).Take(200).Select(r => new JsonObject
            {
                ["member"] = Turn.AddMemberRef(r.UserId, r.UserName),
                ["date"] = Turn.Facts.AddFact("date", DisplayFormat.Day(r.Date, needsYear), resultId),
                ["tracked"] = Turn.Facts.AddFact("tracked time", DisplayFormat.Duration(r.TrackedMinutes), resultId),
                ["first_start"] = Turn.Facts.AddFact("first activity time", DisplayFormat.LocalTime(r.FirstStartedAt, zone), resultId),
                ["last_end"] = Turn.Facts.AddFact("last activity time", DisplayFormat.LocalTime(r.LastEndedAt, zone), resultId)
            });

            return new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, userId),
                ["period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["rows"] = new JsonArray(items.Cast<JsonNode>().ToArray())
            };
        });
    }
}

/// <summary>get_attendance: presence per member over the period.</summary>
internal sealed class GetAttendanceTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetAttendanceTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_attendance";

    protected override string ToolDescription => "Days present against days without tracked time, per member, over the period.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "attendance", null, null, async resultId =>
        {
            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now, "last_7_days");
            var userId = Turn.Caller.CanSeeTeam ? (Guid?)null : Turn.Caller.UserId;
            var attendance = await _read.AttendanceAsync(Turn.Caller, AiToolSupport.Query(period, null, userId, Turn.Caller.TimeZone));

            var rows = attendance.Rows.OrderByDescending(r => r.DaysPresent).Take(50).Select(r => new JsonObject
            {
                ["member"] = Turn.AddMemberRef(r.UserId, r.UserName),
                ["days_present"] = Turn.Facts.AddFact("days present", r.DaysPresent.ToString(System.Globalization.CultureInfo.InvariantCulture), resultId),
                ["days_without_tracked_time"] = Turn.Facts.AddFact(
                    "days without tracked time", r.DaysAbsent.ToString(System.Globalization.CultureInfo.InvariantCulture), resultId),
                ["weekend_days"] = Turn.Facts.AddFact(
                    "weekend days worked", r.WeekendDaysWorked.ToString(System.Globalization.CultureInfo.InvariantCulture), resultId),
                ["worked"] = Turn.Facts.AddFact("tracked time", DisplayFormat.Duration(r.WorkedMinutes), resultId)
            });

            return new JsonObject
            {
                ["scope"] = Turn.Caller.CanSeeTeam ? "team" : "self",
                ["period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["rows"] = new JsonArray(rows.Cast<JsonNode>().ToArray())
            };
        });
    }
}

/// <summary>get_app_usage: focused time per app.</summary>
internal sealed class GetAppUsageTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetAppUsageTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_app_usage";

    protected override string ToolDescription => "Focused time per app, for the period.";

    protected override JsonElement Schema => BuildSchema(("period", true), ("from", true), ("to", true), ("project", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "app usage", null, null, async resultId =>
        {
            var (projectId, projectError) = AiToolGate.ResolveProject(Turn, ReadString(arguments, "project"));
            if (projectError != null)
            {
                return projectError;
            }

            var (userId, memberError) = AiToolGate.ResolveMember(Turn, ReadString(arguments, "member"));
            if (memberError != null)
            {
                return memberError;
            }

            var period = AiToolSupport.ResolvePeriod(arguments, Turn.Caller.TimeZone, _clock.Now);
            var rows = await _read.AppUsageAsync(Turn.Caller, AiToolSupport.Query(period, projectId, userId, Turn.Caller.TimeZone));
            var total = Math.Max(1.0, rows.Sum(r => r.TrackedMinutes));

            var apps = rows.OrderByDescending(r => r.TrackedMinutes).Take(15).Select(r => new JsonObject
            {
                ["app"] = Turn.AddAppRef(r.App),
                ["app_text"] = AiToolSupport.Sanitize(r.App),
                ["tracked"] = Turn.Facts.AddFact("tracked time", DisplayFormat.Duration(r.TrackedMinutes), resultId),
                ["share"] = Turn.Facts.AddFact("share of tracked time", DisplayFormat.Percent(r.TrackedMinutes / total * 100), resultId)
            });

            return new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, userId),
                ["period"] = AiToolSupport.PeriodFact(Turn, period, resultId),
                ["apps"] = new JsonArray(apps.Cast<JsonNode>().ToArray())
            };
        });
    }
}

/// <summary>get_member_day: one local day's segments, first and last, gaps of 10 minutes or more.</summary>
internal sealed class GetMemberDayTool : AiTool
{
    private readonly IAiReadLayer _read;
    private readonly IClock _clock;

    public GetMemberDayTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read, IClock clock) : base(turn, gate)
    {
        _read = read;
        _clock = clock;
    }

    protected override string ToolName => "get_member_day";

    protected override string ToolDescription => "One local day's tracked blocks for one member: first and last activity, and gaps of 10 minutes or more.";

    protected override JsonElement Schema => BuildSchema(("day", true), ("member", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "day detail", AiRouteIds.Activities, null, async resultId =>
        {
            var (userId, memberError) = AiToolGate.ResolveMember(Turn, ReadString(arguments, "member"));
            if (memberError != null)
            {
                return memberError;
            }

            var dayText = ReadString(arguments, "day");
            var zone = ReportCalendar.ResolveZone(Turn.Caller.TimeZone);
            var day = string.IsNullOrEmpty(dayText)
                ? AiPeriodResolver.Resolve(AiPeriodResolver.Today, Turn.Caller.TimeZone, _clock.Now)
                : AiPeriodResolver.ResolveDay(dayText, Turn.Caller.TimeZone);

            // Unlike the other tools, an unspecified member here means the caller's own day, not the team
            // (one calendar day of raw blocks is always about one person).
            var effectiveUserId = userId ?? Turn.Caller.UserId;
            var blocks = await _read.DayBlocksAsync(Turn.Caller, userId, day.FromUtc, day.ToUtc);
            if (blocks.Count == 0)
            {
                var empty = new JsonObject
                {
                    ["scope"] = AiToolGate.ScopeOf(Turn, effectiveUserId),
                    ["day"] = Turn.Facts.AddFact("date", DisplayFormat.Day(day.FirstDay), resultId),
                    ["segments"] = new JsonArray(),
                    ["gaps"] = new JsonArray()
                };
                return empty;
            }

            var ordered = blocks.OrderBy(b => b.StartedAt).ToList();
            var segments = new JsonArray();
            foreach (var block in ordered.Take(40))
            {
                var topApps = block.Apps.OrderByDescending(a => a.Seconds ?? 0).Take(3)
                    .Select(a => (JsonNode)AiToolSupport.Sanitize(a.App)!);
                segments.Add(new JsonObject
                {
                    ["project"] = Turn.AddProjectRef(block.ProjectId),
                    ["start"] = Turn.Facts.AddFact("activity start time", DisplayFormat.LocalTime(block.StartedAt, zone), resultId),
                    ["end"] = Turn.Facts.AddFact("activity end time", DisplayFormat.LocalTime(block.EndedAt, zone), resultId),
                    ["minutes"] = Turn.Facts.AddFact("tracked minutes", DisplayFormat.Duration(block.Minutes), resultId),
                    ["activity_percent"] = Turn.Facts.AddFact("activity percent", DisplayFormat.Percent(block.Productivity), resultId),
                    ["top_apps_text"] = new JsonArray(topApps.ToArray()),
                    ["end_reason_text"] = AiToolSupport.Sanitize(block.EndReason)
                });
            }

            var gaps = new JsonArray();
            for (var i = 1; i < ordered.Count; i++)
            {
                var gapMinutes = (ordered[i].StartedAt - ordered[i - 1].EndedAt).TotalMinutes;
                if (gapMinutes >= 10)
                {
                    gaps.Add(new JsonObject
                    {
                        ["start"] = Turn.Facts.AddFact("gap start time", DisplayFormat.LocalTime(ordered[i - 1].EndedAt, zone), resultId),
                        ["minutes"] = Turn.Facts.AddFact("gap length", DisplayFormat.Duration(gapMinutes), resultId)
                    });
                }
            }

            return new JsonObject
            {
                ["scope"] = AiToolGate.ScopeOf(Turn, effectiveUserId),
                ["day"] = Turn.Facts.AddFact("date", DisplayFormat.Day(day.FirstDay), resultId),
                ["first_start"] = Turn.Facts.AddFact("first activity time", DisplayFormat.LocalTime(ordered[0].StartedAt, zone), resultId),
                ["last_end"] = Turn.Facts.AddFact("last activity time", DisplayFormat.LocalTime(ordered[^1].EndedAt, zone), resultId),
                ["segments"] = segments,
                ["gaps"] = gaps
            };
        });
    }
}

/// <summary>list_projects: the projects the caller may see.</summary>
internal sealed class ListProjectsTool : AiTool
{
    private readonly IAiReadLayer _read;

    public ListProjectsTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read) : base(turn, gate)
    {
        _read = read;
    }

    protected override string ToolName => "list_projects";

    protected override string ToolDescription => "The workspace's projects the person asking may see, with their archived state.";

    protected override JsonElement Schema => BuildSchema(("includeArchived", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "projects", AiRouteIds.Projects, null, async resultId =>
        {
            var includeArchived = string.Equals(ReadString(arguments, "includeArchived"), "true", StringComparison.OrdinalIgnoreCase);
            var projects = await _read.ProjectsAsync(Turn.Caller, includeArchived);

            var items = projects.Take(100).Select(p => new JsonObject
            {
                ["project"] = Turn.AddProjectRef(p.Id, p.Title),
                ["title_text"] = AiToolSupport.Sanitize(p.Title),
                ["archived"] = p.IsArchived
            });

            return new JsonObject { ["projects"] = new JsonArray(items.Cast<JsonNode>().ToArray()) };
        });
    }
}

/// <summary>get_project_tasks: one project's tasks, without descriptions.</summary>
internal sealed class GetProjectTasksTool : AiTool
{
    private readonly IAiReadLayer _read;

    public GetProjectTasksTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read) : base(turn, gate)
    {
        _read = read;
    }

    protected override string ToolName => "get_project_tasks";

    protected override string ToolDescription => "One project's tasks (titles only, no descriptions), optionally filtered to open or completed.";

    protected override JsonElement Schema => BuildSchema(("project", false), ("status", true));

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "project tasks", AiRouteIds.ProjectDetail, null, async resultId =>
        {
            var (projectId, projectError) = AiToolGate.ResolveProject(Turn, ReadString(arguments, "project"));
            if (projectError != null)
            {
                return projectError;
            }

            if (projectId == null)
            {
                return AiToolErrors.Of(AiToolErrors.UnknownProject);
            }

            var status = ReadString(arguments, "status");
            bool? isCompleted = status switch
            {
                "open" => false,
                "completed" => true,
                _ => null
            };

            var page = await _read.TasksAsync(Turn.Caller, projectId.Value, isCompleted, 100);
            var items = page.Items.Select(t => new JsonObject
            {
                ["task"] = Turn.AddTaskRef(t.Id, t.Title),
                ["title_text"] = AiToolSupport.Sanitize(t.Title),
                ["completed"] = t.IsCompleted,
                ["completed_date"] = t.CompletedAt == null
                    ? null
                    : Turn.Facts.AddFact("completed date", DisplayFormat.Day(t.CompletedAt.Value), resultId)
            });

            return new JsonObject
            {
                ["total_count"] = page.TotalCount,
                ["tasks"] = new JsonArray(items.Cast<JsonNode>().ToArray())
            };
        });
    }
}

/// <summary>list_team: the workspace's active members (owner and manager only).</summary>
internal sealed class ListTeamTool : AiTool
{
    private readonly IAiReadLayer _read;

    public ListTeamTool(AiTurnContext turn, AiToolGate gate, IAiReadLayer read) : base(turn, gate)
    {
        _read = read;
    }

    protected override string ToolName => "list_team";

    protected override string ToolDescription => "The workspace's active members, their role and when they joined. No emails.";

    protected override JsonElement Schema => EmptySchema();

    protected override async Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return await Gate.RunAsync(Turn, ToolName, "team", AiRouteIds.Team, null, async resultId =>
        {
            var roster = await _read.RosterAsync(Turn.Caller);
            var items = roster.Select(m => new JsonObject
            {
                ["member"] = Turn.AddMemberRef(m.UserId, m.DisplayName),
                ["role"] = m.Role,
                ["joined"] = Turn.Facts.AddFact("the day they joined", DisplayFormat.Day(DateTime.SpecifyKind(m.JoinedAt.Date, DateTimeKind.Unspecified)), resultId)
            });

            return new JsonObject { ["members"] = new JsonArray(items.Cast<JsonNode>().ToArray()) };
        });
    }
}

/// <summary>get_help: the product's own how-to text. Carries no workspace data; every role may call it.</summary>
internal sealed class GetHelpTool : AiTool
{
    private readonly AiHelpLibrary _help;

    public GetHelpTool(AiTurnContext turn, AiToolGate gate, AiHelpLibrary help) : base(turn, gate)
    {
        _help = help;
    }

    protected override string ToolName => "get_help";

    protected override string ToolDescription => "Dosi-Tracker's own help text: how to install the agent, invite people, use projects, reports and so on.";

    protected override JsonElement Schema => BuildSchema(("topic", false));

    protected override Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return Gate.RunAsync(Turn, ToolName, "help", null, null, resultId =>
        {
            var topic = ReadString(arguments, "topic") ?? string.Empty;
            var articles = _help.Find(topic);
            var items = articles.Select(a => new JsonObject
            {
                ["title_text"] = AiToolSupport.Sanitize(a.Title),
                ["help_text"] = AiToolSupport.Sanitize(a.Body)
            });

            return Task.FromResult(new JsonObject { ["articles"] = new JsonArray(items.Cast<JsonNode>().ToArray()) });
        });
    }
}
