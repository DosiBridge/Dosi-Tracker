using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Users;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>
/// <see cref="IAiReadLayer"/> over the product's app services. It depends on app-service interfaces and the extracted
/// note rule only (an architecture test keeps it so), never on a repository or the database, so AI assist can read
/// nothing the caller could not read through the API.
/// </summary>
public class AiReadLayer : IAiReadLayer, ITransientDependency
{
    /// <summary>The most blocks one day read returns (the activity list's page limit).</summary>
    public const int MaxDayBlocks = 1000;

    /// <summary>A local day lasts at most 25 hours (a DST change); a day read may span this much.</summary>
    public static readonly TimeSpan MaxDaySpan = TimeSpan.FromHours(26);

    public const int MaxProjects = 1000;
    public const int MaxTasks = 100;

    private readonly IReportingAppService _reporting;
    private readonly IActivityAppService _activities;
    private readonly ITeamAppService _team;
    private readonly IProjectAppService _projects;
    private readonly IProjectTaskAppService _tasks;
    private readonly ActivityNoteRules _noteRules;
    private readonly ICurrentUser _currentUser;
    private readonly ICurrentWorkspace _currentWorkspace;

    public AiReadLayer(
        IReportingAppService reporting,
        IActivityAppService activities,
        ITeamAppService team,
        IProjectAppService projects,
        IProjectTaskAppService tasks,
        ActivityNoteRules noteRules,
        ICurrentUser currentUser,
        ICurrentWorkspace currentWorkspace)
    {
        _reporting = reporting;
        _activities = activities;
        _team = team;
        _projects = projects;
        _tasks = tasks;
        _noteRules = noteRules;
        _currentUser = currentUser;
        _currentWorkspace = currentWorkspace;
    }

    public virtual async Task<AiSummary> SummaryAsync(AiCallerContext caller, AiQuery query)
    {
        var summary = await _reporting.GetSummaryAsync(ToReportInput(caller, query));
        var names = summary.PerUser.ToDictionary(u => u.UserId, DisplayName);

        return new AiSummary(
            summary.TotalActivities,
            summary.TotalTrackedMinutes,
            summary.TotalIdleMinutes,
            summary.AverageProductivity,
            summary.PerUser
                .Select(u => new AiMemberRow(u.UserId, DisplayName(u), u.ActivityCount, u.TrackedMinutes, u.IdleMinutes, u.AverageProductivity))
                .ToList(),
            summary.PerProject
                .Select(p => new AiProjectRow(p.ProjectId, p.ProjectTitle, p.IsArchived, p.ActivityCount, p.TrackedMinutes, p.IdleMinutes, p.AverageProductivity))
                .ToList(),
            summary.PerUserProject
                .Select(r => new AiMemberProjectRow(
                    r.UserId, names.TryGetValue(r.UserId, out var name) ? name : r.UserName, r.ProjectId, r.ProjectTitle,
                    r.ActivityCount, r.TrackedMinutes, r.IdleMinutes, r.AverageProductivity))
                .ToList());
    }

    public virtual async Task<IReadOnlyList<AiDailyRow>> DailyAsync(AiCallerContext caller, AiQuery query)
    {
        return (await _reporting.GetDailySeriesAsync(ToReportInput(caller, query)))
            .Select(d => new AiDailyRow(d.Date, d.ActivityCount, d.TrackedMinutes, d.IdleMinutes, d.AverageProductivity))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiMemberDayRow>> UserDailyAsync(AiCallerContext caller, AiQuery query)
    {
        return (await _reporting.GetUserDailySeriesAsync(ToReportInput(caller, query)))
            .Select(d => new AiMemberDayRow(d.UserId, d.UserName, d.Date, d.ActivityCount, d.TrackedMinutes, d.IdleMinutes,
                d.FirstStartedAt, d.LastEndedAt))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiAppUsageRow>> AppUsageAsync(AiCallerContext caller, AiQuery query)
    {
        return (await _reporting.GetAppUsageAsync(ToReportInput(caller, query)))
            .Select(a => new AiAppUsageRow(
                string.IsNullOrWhiteSpace(a.DisplayName) ? AppNames.Canonical(a.AppName) : a.DisplayName,
                a.TrackedMinutes, a.IdleMinutes, a.ActivityCount, a.UserCount))
            .ToList();
    }

    public virtual async Task<AiAttendance> AttendanceAsync(AiCallerContext caller, AiQuery query)
    {
        var attendance = await _reporting.GetAttendanceAsync(ToReportInput(caller, query));
        return new AiAttendance(
            attendance.Days.ToList(),
            attendance.Rows
                .Select(r => new AiAttendanceRow(r.UserId, r.UserName, r.DaysPresent, r.DaysAbsent, r.WeekendDaysWorked,
                    r.WorkedMinutes, r.IdleMinutes, r.AverageStartMinutes, r.AverageEndMinutes))
                .ToList());
    }

    public virtual async Task<IReadOnlyList<AiHourlyRow>> HourlyAsync(AiCallerContext caller, AiQuery query)
    {
        return (await _reporting.GetHourlyAsync(ToReportInput(caller, query)))
            .Select(h => new AiHourlyRow(h.DayOfWeek, h.Hour, h.TrackedMinutes, h.ActiveMinutes))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiDeviceRow>> DevicesAsync(AiCallerContext caller, AiQuery query)
    {
        // Device names and OS versions are dropped here, at the source.
        return (await _reporting.GetDevicesAsync(ToReportInput(caller, query)))
            .Select(d => new AiDeviceRow(d.UserId, d.UserName, d.DeviceId, d.Platform, d.AgentVersion, d.LastActivityAt, d.TrackedMinutes))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiDataQualityRow>> DataQualityAsync(AiCallerContext caller, AiQuery query)
    {
        return (await _reporting.GetDataQualityAsync(ToReportInput(caller, query)))
            .Select(d => new AiDataQualityRow(d.UserId, d.UserName, d.Blocks, d.TrackedMinutes, d.UnmeasuredMinutes,
                d.MaxAbsClockSkewSeconds, d.BlocksWithInjectedInput, d.LastActivityAt))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiBlock>> DayBlocksAsync(AiCallerContext caller, Guid? userId, DateTime fromUtc, DateTime toUtc)
    {
        EnsureCaller(caller);
        if (toUtc <= fromUtc)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid);
        }

        if (toUtc - fromUtc > MaxDaySpan)
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiRangeTooLarge).WithData("maxDays", 1);
        }

        var page = await _activities.GetListAsync(new GetActivitiesInput
        {
            UserId = ScopedUserId(caller, userId ?? caller.UserId),
            From = fromUtc,
            To = toUtc,
            MaxResultCount = MaxDayBlocks
        });

        return page.Items
            .OrderBy(a => a.StartedAt)
            .ThenBy(a => a.Id)
            .Select(a => ToBlock(caller, a.Id, a.UserId, a.ProjectId, a.StartedAt, a.EndedAt, a.Productivity, a.IdleSeconds,
                a.EndReason, a.ClockSkewSeconds, a.InjectedInputEvents, a.ActiveWindowsJson, a.Description))
            .ToList();
    }

    public virtual async Task<IReadOnlyList<AiRosterEntry>> RosterAsync(AiCallerContext caller)
    {
        EnsureCaller(caller);
        var members = (await _team.GetMembersAsync()).Items;

        // Only a caller who sees the team gets the team (a manager without Reporting still gets just themselves).
        return members
            .Where(m => caller.CanSeeTeam || m.UserId == caller.UserId)
            .Select(m => new AiRosterEntry(m.UserId, m.UserName, m.Name, m.Surname, m.Role, m.IsClient, m.JoinedAt))
            .ToList();
    }

    public virtual async Task<AiProject> ProjectAsync(AiCallerContext caller, Guid projectId)
    {
        EnsureCaller(caller);
        var project = await _projects.GetAsync(projectId);
        return new AiProject(project.Id, project.Title, project.Description, project.IsArchived);
    }

    public virtual async Task<IReadOnlyList<AiProject>> ProjectsAsync(AiCallerContext caller, bool includeArchived)
    {
        EnsureCaller(caller);
        var page = await _projects.GetListAsync(new PagedAndSortedResultRequestDto { MaxResultCount = MaxProjects });
        return page.Items
            .Where(p => includeArchived || !p.IsArchived)
            .Select(p => new AiProject(p.Id, p.Title, p.Description, p.IsArchived))
            .ToList();
    }

    public virtual async Task<AiTaskPage> TasksAsync(AiCallerContext caller, Guid projectId, bool? isCompleted, int max)
    {
        EnsureCaller(caller);
        var page = await _tasks.GetListAsync(new GetProjectTasksInput
        {
            ProjectId = projectId,
            IsCompleted = isCompleted,
            MaxResultCount = Math.Clamp(max, 1, MaxTasks)
        });

        return new AiTaskPage(
            page.TotalCount,
            page.Items.Select(t => new AiTask(t.Id, t.Title, t.IsCompleted, t.CompletedAt)).ToList());
    }

    public virtual async Task<AiBlock> NoteBlockAsync(AiCallerContext caller, Guid activityId)
    {
        EnsureCaller(caller);
        var a = await _noteRules.LoadForSuggestionAsync(activityId);
        return ToBlock(caller, a.Id, a.UserId, a.ProjectId, a.StartedAt, a.EndedAt, a.Productivity, a.IdleSeconds,
            a.EndReason, a.ClockSkewSeconds, a.InjectedInputEvents, a.ActiveWindowsJson, a.Description);
    }

    /// <summary>The reporting input for <paramref name="query"/>, scoped (see the interface remarks).</summary>
    protected virtual GetReportSummaryInput ToReportInput(AiCallerContext caller, AiQuery query)
    {
        EnsureCaller(caller);
        Check.NotNull(query, nameof(query));

        return new GetReportSummaryInput
        {
            From = query.From,
            To = query.To,
            ProjectId = query.ProjectId,
            UserId = ScopedUserId(caller, query.UserId),
            TimeZone = string.IsNullOrWhiteSpace(query.TimeZone) ? caller.TimeZone : query.TimeZone,
            DayBasis = query.DayBasis
        };
    }

    /// <summary>The member a read is about: as asked for a caller who sees the team (null: the team); always the caller
    /// otherwise, and another member is refused rather than replaced.</summary>
    protected static Guid? ScopedUserId(AiCallerContext caller, Guid? requested)
    {
        if (caller.CanSeeTeam)
        {
            return requested;
        }

        if (requested.HasValue && requested.Value != caller.UserId)
        {
            throw new AbpAuthorizationException(AiScopeGuard.TeamScopeMessage);
        }

        return caller.UserId;
    }

    private static AiBlock ToBlock(
        AiCallerContext caller,
        Guid id,
        Guid userId,
        Guid projectId,
        DateTime startedAt,
        DateTime endedAt,
        int? productivity,
        int? idleSeconds,
        string? endReason,
        int? clockSkewSeconds,
        int? injectedInputEvents,
        string? activeWindowsJson,
        string? description)
    {
        var windows = AiActivityParsing.Parse(activeWindowsJson, caller.Policy.AllowWindowTitles);
        return new AiBlock(
            id,
            userId,
            projectId,
            startedAt,
            endedAt,
            productivity,
            idleSeconds,
            endReason,
            clockSkewSeconds,
            injectedInputEvents,
            windows.Apps,
            caller.Policy.AllowWindowTitles ? windows.Titles : null,
            caller.Policy.AllowNotes && !string.IsNullOrWhiteSpace(description) ? description.Trim() : null);
    }

    private static string DisplayName(UserReportRowDto row) =>
        string.IsNullOrWhiteSpace(row.Name) ? row.UserName : row.Name.Trim();

    /// <summary>The context must be the signed-in caller's in the current workspace: the app services read as them.</summary>
    private void EnsureCaller(AiCallerContext caller)
    {
        Check.NotNull(caller, nameof(caller));
        if (_currentUser.Id != caller.UserId || _currentWorkspace.Id != caller.WorkspaceId)
        {
            throw new InvalidOperationException("An AI read must run as the caller it was authorized for, in their workspace.");
        }
    }
}
