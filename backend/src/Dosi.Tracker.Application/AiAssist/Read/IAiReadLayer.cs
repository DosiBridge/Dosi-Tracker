using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>
/// The only way AI features and Ask AI tools read workspace data (spec 10.1). Every method calls the product's own app
/// services as the signed-in caller, so their authorization, scoping and validation run unchanged, and returns
/// minimized read models (<see cref="AiSummary"/>, <see cref="AiBlock"/> …). It never writes.
/// <para>
/// Scope: a caller who sees the team (<see cref="AiCallerContext.CanSeeTeam"/>) reads what the query names (the team
/// when it names no member). Anyone else reads their own data only: the member is always set to the caller, so a
/// client of a project never receives other members' figures in a "self" answer, and naming another member is refused
/// with an <c>AbpAuthorizationException</c> instead of being quietly narrowed. Callers still run
/// <see cref="AiScopeGuard"/> first.
/// </para>
/// <para>Window titles and notes are read only when the workspace allows them (<see cref="AiWorkspacePolicy"/>); otherwise
/// they are null at the source.</para>
/// </summary>
public interface IAiReadLayer
{
    /// <summary>Totals per member, project and member × project (IReportingAppService.GetSummaryAsync).</summary>
    Task<AiSummary> SummaryAsync(AiCallerContext caller, AiQuery query);

    /// <summary>One row per local day of the range (GetDailySeriesAsync).</summary>
    Task<IReadOnlyList<AiDailyRow>> DailyAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Member-days with tracked time (GetUserDailySeriesAsync).</summary>
    Task<IReadOnlyList<AiMemberDayRow>> UserDailyAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Focused time per app (GetAppUsageAsync).</summary>
    Task<IReadOnlyList<AiAppUsageRow>> AppUsageAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Presence per member (GetAttendanceAsync).</summary>
    Task<AiAttendance> AttendanceAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Tracked and active minutes per weekday and hour (GetHourlyAsync).</summary>
    Task<IReadOnlyList<AiHourlyRow>> HourlyAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Devices per member: platform and agent version only (GetDevicesAsync).</summary>
    Task<IReadOnlyList<AiDeviceRow>> DevicesAsync(AiCallerContext caller, AiQuery query);

    /// <summary>Data-quality signals per member (GetDataQualityAsync).</summary>
    Task<IReadOnlyList<AiDataQualityRow>> DataQualityAsync(AiCallerContext caller, AiQuery query);

    /// <summary>
    /// The blocks of one member that started in [<paramref name="fromUtc"/>, <paramref name="toUtc"/>), at most one
    /// local day and 1000 blocks, oldest first (IActivityAppService.GetListAsync).
    /// </summary>
    /// <param name="userId">The member; the caller when null.</param>
    Task<IReadOnlyList<AiBlock>> DayBlocksAsync(AiCallerContext caller, Guid? userId, DateTime fromUtc, DateTime toUtc);

    /// <summary>The workspace's active members the caller may see (ITeamAppService.GetMembersAsync), without emails.</summary>
    Task<IReadOnlyList<AiRosterEntry>> RosterAsync(AiCallerContext caller);

    /// <summary>One project the caller may see (IProjectAppService.GetAsync; 404 otherwise).</summary>
    Task<AiProject> ProjectAsync(AiCallerContext caller, Guid projectId);

    /// <summary>The projects the caller may see, at most 1000 (IProjectAppService.GetListAsync).</summary>
    Task<IReadOnlyList<AiProject>> ProjectsAsync(AiCallerContext caller, bool includeArchived);

    /// <summary>A project's tasks, newest first, at most <paramref name="max"/> (≤ 100) of them
    /// (IProjectTaskAppService.GetListAsync).</summary>
    Task<AiTaskPage> TasksAsync(AiCallerContext caller, Guid projectId, bool? isCompleted, int max);

    /// <summary>The block a note is suggested for, by the rules of changing its note (ActivityNoteRules): the caller's own
    /// block within the edit window, or another member's for a manager who may also read it.</summary>
    Task<AiBlock> NoteBlockAsync(AiCallerContext caller, Guid activityId);
}
