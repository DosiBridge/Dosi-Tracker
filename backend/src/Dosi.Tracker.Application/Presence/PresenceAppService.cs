using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Auditing;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Volo.Abp.Users;

using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Presence;

/// <summary>
/// Live "who is working now" (shared contract §3). Every running agent reports its state with a heartbeat; there is
/// one <see cref="AgentPresence"/> row per (workspace, member, device install), and a device counts as online while
/// its latest heartbeat is at most <see cref="AgentPresenceConsts.OnlineWindowSeconds"/> old. A member keeps at most
/// <see cref="AgentPresenceConsts.MaxDevicesPerUser"/> devices, none of them unseen for
/// <see cref="AgentPresenceConsts.StaleDeviceDays"/> days: the list is tidied whenever a new device of theirs reports.
/// </summary>
[Authorize]
public class PresenceAppService : TrackerAppService, IPresenceAppService
{
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;

    public PresenceAppService(
        IRepository<AgentPresence, Guid> presenceRepository,
        IRepository<Project, Guid> projectRepository,
        IIdentityUserRepository userRepository,
        IRepository<WorkspaceMember, Guid> memberRepository)
    {
        _presenceRepository = presenceRepository;
        _projectRepository = projectRepository;
        _userRepository = userRepository;
        _memberRepository = memberRepository;
    }

    /// <summary>Records the caller's agent state on one device. Not written to the audit log: every running agent
    /// sends one a minute, and the foreground application is not audit material.</summary>
    [DisableAuditing]
    public async Task<HeartbeatResultDto> HeartbeatAsync(HeartbeatDto input)
    {
        var userId = CurrentUser.GetId();
        var now = AgentContextNormalizer.AsUtc(Clock.Now);
        // Validation has already refused an unknown status (400).
        var status = HeartbeatDto.FindStatus(input.Status) ?? throw new ArgumentException("Unknown status.", nameof(input));

        // A project of another workspace, or a deleted one, is not found and the heartbeat carries no project.
        var project = input.ProjectId is { } projectId && projectId != Guid.Empty
            ? await _projectRepository.FindAsync(projectId)
            : null;

        var presence = await FindDeviceAsync(userId, input.DeviceId);
        if (presence == null)
        {
            var created = new AgentPresence(GuidGenerator.Create(), CurrentWorkspace.GetId(), userId, input.DeviceId, status, now);
            Apply(created, input, status, project, now, previousStatus: null);

            // Null when a concurrent heartbeat of the same device created the row first; that row is updated instead.
            presence = await InsertOrFindConcurrentAsync(created);
            if (presence == null)
            {
                await RemoveSurplusDevicesAsync(userId, created.Id, now);
                return new HeartbeatResultDto { ServerTime = now };
            }
        }

        Apply(presence, input, status, project, now, presence.Status);
        await _presenceRepository.UpdateAsync(presence);

        return new HeartbeatResultDto { ServerTime = now };
    }

    public async Task<ListResultDto<PresenceDto>> GetTeamAsync()
    {
        var currentWorkspaceId = CurrentWorkspace.GetId();
        var memberQuery = await _memberRepository.GetQueryableAsync();
        var activeMemberUserIds = memberQuery
            .Where(m => m.WorkspaceId == currentWorkspaceId && m.Status == WorkspaceMemberStatus.Active)
            .Select(m => m.UserId);

        var query = (await _presenceRepository.GetQueryableAsync())
            .Where(p => activeMemberUserIds.Contains(p.UserId));

        if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            // Members only ever see their own devices.
            var userId = CurrentUser.GetId();
            query = query.Where(p => p.UserId == userId);
        }

        var rows = await AsyncExecuter.ToListAsync(query);
        if (rows.Count == 0)
        {
            return new ListResultDto<PresenceDto>(new List<PresenceDto>());
        }

        // Only devices of current members: a deleted or deactivated account is no longer part of the team.
        var users = (await _userRepository.GetListByIdsAsync(rows.Select(r => r.UserId).Distinct()))
            .Where(u => u.IsActive)
            .ToDictionary(u => u.Id);

        var projectIds = rows.Where(r => r.ProjectId.HasValue).Select(r => r.ProjectId!.Value).Distinct().ToList();
        var projects = projectIds.Count == 0
            ? new Dictionary<Guid, Project>()
            : (await _projectRepository.GetListAsync(p => projectIds.Contains(p.Id))).ToDictionary(p => p.Id);

        var now = AgentContextNormalizer.AsUtc(Clock.Now);
        var items = rows
            .Where(r => users.ContainsKey(r.UserId))
            .Select(r => MapToDto(
                r,
                users[r.UserId],
                r.ProjectId.HasValue ? projects.GetValueOrDefault(r.ProjectId.Value) : null,
                now))
            .OrderByDescending(p => p.Online)
            .ThenBy(p => p.Name ?? p.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(p => p.DeviceName ?? string.Empty, StringComparer.OrdinalIgnoreCase)
            .ThenBy(p => p.DeviceId)
            .ToList();

        return new ListResultDto<PresenceDto>(items);
    }

    private Task<AgentPresence?> FindDeviceAsync(Guid userId, Guid deviceId) =>
        _presenceRepository.FirstOrDefaultAsync(p => p.UserId == userId && p.DeviceId == deviceId);

    /// <summary>
    /// Bounds a member's device list once a new device of theirs (<paramref name="newDeviceRowId"/>) has reported: their
    /// other devices not seen for <see cref="AgentPresenceConsts.StaleDeviceDays"/> days, and all but the most recently
    /// seen ones beyond <see cref="AgentPresenceConsts.MaxDevicesPerUser"/>, are removed. Reinstalls (a new random device
    /// id each time), or a script inventing device ids, would otherwise add rows — and devices to every manager's live
    /// view — without end. Removed by a set-based delete, so a concurrent tidy-up of the same rows cannot fail it.
    /// </summary>
    private async Task RemoveSurplusDevicesAsync(Guid userId, Guid newDeviceRowId, DateTime now)
    {
        var others = await AsyncExecuter.ToListAsync((await _presenceRepository.GetQueryableAsync())
            .Where(p => p.UserId == userId && p.Id != newDeviceRowId)
            .Select(p => new { p.Id, p.LastSeenAt, p.CreationTime }));

        var staleBefore = now.AddDays(-AgentPresenceConsts.StaleDeviceDays);
        var surplus = others
            .OrderByDescending(p => AgentContextNormalizer.AsUtc(p.LastSeenAt))
            .ThenByDescending(p => p.CreationTime)
            .ThenBy(p => p.Id)
            .Select((p, rank) => (p.Id, Keep: rank < AgentPresenceConsts.MaxDevicesPerUser - 1 &&
                                             AgentContextNormalizer.AsUtc(p.LastSeenAt) >= staleBefore))
            .Where(p => !p.Keep)
            .Select(p => p.Id)
            .ToList();

        if (surplus.Count > 0)
        {
            await _presenceRepository.DeleteDirectAsync(p => surplus.Contains(p.Id));
        }
    }

    /// <summary>Inserts the first row of a device in a unit of work of its own (a refused insert must not poison the
    /// caller's). When the unique (WorkspaceId, UserId, DeviceId) index refuses it because a concurrent heartbeat
    /// stored the device first, returns that row; returns null when this insert won.</summary>
    private async Task<AgentPresence?> InsertOrFindConcurrentAsync(AgentPresence presence)
    {
        try
        {
            using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
            await _presenceRepository.InsertAsync(presence);
            await uow.CompleteAsync();
            return null;
        }
        catch (Exception exception) when (DatabaseErrors.MayBeUniqueViolation(exception))
        {
            var existing = await FindDeviceAsync(presence.UserId, presence.DeviceId);
            if (existing == null)
            {
                throw;
            }

            return existing;
        }
    }

    /// <summary>
    /// Applies a heartbeat. The status, project and foreground application describe this moment and are always
    /// replaced; the device descriptors (name, platform, versions, time zone) are only replaced by a value the agent
    /// sent, so a minimal heartbeat (e.g. "stopped" on quit) keeps them. The foreground application is kept only when
    /// the project allows the active window. <see cref="AgentPresence.StatusSince"/> is brought onto the server's
    /// clock with the device's skew (from <see cref="HeartbeatDto.SentAt"/>); without one from the agent, a new or
    /// changed status starts now.
    /// </summary>
    private static void Apply(
        AgentPresence presence, HeartbeatDto input, string status, Project? project, DateTime now, string? previousStatus)
    {
        presence.Status = status;
        presence.LastSeenAt = now;
        presence.ProjectId = project?.Id;
        presence.CurrentApp = project?.AllowActiveWindow == true
            ? AgentContextNormalizer.CleanText(input.CurrentApp, AgentPresenceConsts.MaxCurrentAppLength)
            : null;

        var statusSince = ToServerClock(input.StatusSince, input.SentAt, now);
        if (statusSince.HasValue)
        {
            presence.StatusSince = statusSince;
        }
        else if (!string.Equals(previousStatus, status, StringComparison.Ordinal) || presence.StatusSince == null)
        {
            presence.StatusSince = now;
        }

        presence.DeviceName = AgentContextNormalizer.CleanText(input.DeviceName, ActivityConsts.MaxDeviceNameLength) ?? presence.DeviceName;
        presence.Platform = AgentContextNormalizer.NormalizePlatform(input.Platform) ?? presence.Platform;
        presence.OsVersion = AgentContextNormalizer.CleanText(input.OsVersion, ActivityConsts.MaxOsVersionLength) ?? presence.OsVersion;
        presence.AgentVersion = AgentContextNormalizer.CleanText(input.AgentVersion, ActivityConsts.MaxAgentVersionLength) ?? presence.AgentVersion;
        presence.TimeZone = AgentContextNormalizer.NormalizeTimeZone(input.TimeZone) ?? presence.TimeZone;
        presence.UtcOffsetMinutes = AgentContextNormalizer.NormalizeUtcOffset(input.UtcOffsetMinutes) ?? presence.UtcOffsetMinutes;
    }

    /// <summary>A device timestamp moved onto the server's clock by the device's skew, and never later than now.</summary>
    private static DateTime? ToServerClock(DateTime? deviceTime, DateTime? sentAt, DateTime now)
    {
        if (!deviceTime.HasValue)
        {
            return null;
        }

        var skew = TimeSpan.FromSeconds(AgentContextNormalizer.ClockSkewSeconds(now, sentAt) ?? 0);
        var value = Shift(AgentContextNormalizer.AsUtc(deviceTime.Value), skew);

        return value > now ? now : value;
    }

    /// <summary><paramref name="value"/> + <paramref name="offset"/>, saturating at the bounds of <see cref="DateTime"/>.</summary>
    private static DateTime Shift(DateTime value, TimeSpan offset)
    {
        if (offset >= TimeSpan.Zero)
        {
            return DateTime.MaxValue - value <= offset ? DateTime.SpecifyKind(DateTime.MaxValue, DateTimeKind.Utc) : value + offset;
        }

        return value - DateTime.MinValue <= offset.Duration() ? DateTime.SpecifyKind(DateTime.MinValue, DateTimeKind.Utc) : value + offset;
    }

    private static PresenceDto MapToDto(AgentPresence presence, IdentityUser user, Project? project, DateTime now)
    {
        var fullName = string.Join(" ",
            new[] { user.Name, user.Surname }.Where(s => !string.IsNullOrWhiteSpace(s)).Select(s => s!.Trim()));

        return new PresenceDto
        {
            UserId = presence.UserId,
            UserName = user.UserName,
            Name = fullName.Length == 0 ? null : fullName,
            DeviceId = presence.DeviceId,
            DeviceName = presence.DeviceName,
            Platform = presence.Platform,
            OsVersion = presence.OsVersion,
            AgentVersion = presence.AgentVersion,
            Status = presence.Status,
            ProjectId = presence.ProjectId,
            ProjectTitle = project?.Title,
            StatusSince = presence.StatusSince,
            LastSeenAt = presence.LastSeenAt,
            Online = now - AgentContextNormalizer.AsUtc(presence.LastSeenAt) <= TimeSpan.FromSeconds(AgentPresenceConsts.OnlineWindowSeconds),
            // The project's current policy applies to what was stored, so turning the active window off hides it at once.
            CurrentApp = project?.AllowActiveWindow == true ? presence.CurrentApp : null,
            TimeZone = presence.TimeZone,
            UtcOffsetMinutes = presence.UtcOffsetMinutes
        };
    }
}
