using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.Logging;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Microsoft.AspNetCore.Identity;
using Volo.Abp.Identity;
using Volo.Abp.Uow;

using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Erasure;

/// <summary>
/// Self-service account deletion. <see cref="DeleteAsync"/> runs while the person waits: the account leaves every
/// workspace, every sign-in session is revoked, its security log goes, and the identity is anonymized (no name, email
/// or phone; the address can register again) and deleted. The tracked data is scrubbed afterwards by the erasure sweep
/// (<see cref="ScrubAsync"/>): every capture is deleted with its file, and every time block loses its window titles,
/// application names, note and device name. What stays of a block is the work-time record its workspace keeps — when,
/// on which project, the input counts — attributed to a deleted account.
/// </summary>
public class AccountEraser : TrackerDomainService
{
    /// <summary>The domain of the placeholder address a deleted account keeps (reserved: it never receives mail).</summary>
    public const string PlaceholderEmailDomain = "deleted.invalid";

    private readonly IdentityUserManager _userManager;
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<AccountErasure, Guid> _erasureRepository;
    private readonly WorkspaceMembershipManager _memberships;
    private readonly CaptureEraser _captureEraser;

    public AccountEraser(
        IdentityUserManager userManager,
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IRepository<Workspace, Guid> workspaceRepository,
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<AccountErasure, Guid> erasureRepository,
        WorkspaceMembershipManager memberships,
        CaptureEraser captureEraser)
    {
        _userManager = userManager;
        _workspaceMemberRepository = workspaceMemberRepository;
        _workspaceRepository = workspaceRepository;
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _erasureRepository = erasureRepository;
        _memberships = memberships;
        _captureEraser = captureEraser;
    }

    /// <summary>The workspaces (not deleted) the user is the only active owner of: while there is one, the account cannot
    /// be deleted — the workspace would be left without anyone able to manage it.</summary>
    public virtual async Task<List<(Guid Id, string Name)>> FindSoleOwnedWorkspacesAsync(Guid userId)
    {
        var members = await _workspaceMemberRepository.GetQueryableAsync();
        var workspaces = await _workspaceRepository.GetQueryableAsync();
        var owned = await AsyncExecuter.ToListAsync(
            from m in members
            join w in workspaces on m.WorkspaceId equals w.Id
            where m.UserId == userId && m.Role == WorkspaceRoles.Owner && m.Status == WorkspaceMemberStatus.Active &&
                  !members.Any(o => o.WorkspaceId == m.WorkspaceId && o.UserId != userId &&
                                    o.Role == WorkspaceRoles.Owner && o.Status == WorkspaceMemberStatus.Active)
            orderby w.Name
            select new { w.Id, w.Name });

        return owned.Select(w => (w.Id, w.Name)).ToList();
    }

    /// <summary>
    /// Deletes the account <paramref name="userId"/>; refused with <c>Tracker:SoleWorkspaceOwner</c> while it is the
    /// only owner of a workspace. Holds the owner lock of every workspace it owns, so no other owner can leave in the
    /// meantime. Saved with the caller's unit of work (make it transactional); the scrub of its tracked data is queued.
    /// </summary>
    public virtual async Task DeleteAsync(Guid userId)
    {
        var user = await _userManager.GetByIdAsync(userId);

        var memberships = await _workspaceMemberRepository.GetListAsync(m =>
            m.UserId == userId && m.Status == WorkspaceMemberStatus.Active);
        var locks = new List<IAsyncDisposable>();
        try
        {
            foreach (var owned in memberships.Where(m => m.IsOwner).OrderBy(m => m.WorkspaceId))
            {
                locks.Add(await _memberships.LockOwnersAsync(owned.WorkspaceId));
            }

            var soleOwned = await FindSoleOwnedWorkspacesAsync(userId);
            if (soleOwned.Count > 0)
            {
                throw new BusinessException(TrackerDomainErrorCodes.SoleWorkspaceOwner)
                    .WithData("workspaces", string.Join(", ", soleOwned.Select(w => w.Name)));
            }

            await LeaveEverythingAsync(userId, memberships);
            await LazyServiceProvider.LazyGetRequiredService<DeviceSessionManager>().RevokeAllAsync(userId);
            await LazyServiceProvider.LazyGetRequiredService<IRepository<IdentitySecurityLog, Guid>>()
                .DeleteDirectAsync(log => log.UserId == userId);
            await AnonymizeAsync(user);

            await _erasureRepository.InsertAsync(new AccountErasure(GuidGenerator.Create(), userId), autoSave: true);
            Logger.LogInformation("Account {UserId} was deleted by its owner; its tracked data will be scrubbed.", userId);
        }
        finally
        {
            foreach (var handle in locks)
            {
                await handle.DisposeAsync();
            }
        }
    }

    /// <summary>
    /// Scrubs the tracked data of a deleted account, page by page: deletes its captures with their files, then clears
    /// the window titles, application names, notes and device names of its blocks (in every workspace). True once nothing
    /// is left to scrub, after which the request is marked completed; false while a capture file could not be deleted
    /// (the next sweep tries again).
    /// </summary>
    public virtual async Task<bool> ScrubAsync(Guid erasureId, int batchSize, CancellationToken cancellationToken = default)
    {
        batchSize = Math.Max(1, batchSize);
        Guid userId;
        using (var uow = UnitOfWorkManager.Begin(requiresNew: true))
        {
            var request = await _erasureRepository.GetAsync(erasureId, cancellationToken: cancellationToken);
            await uow.CompleteAsync(cancellationToken);
            if (request.CompletedTime != null)
            {
                return true;
            }

            userId = request.UserId;
        }

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            if (!await DeleteCapturesAsync(userId, batchSize, cancellationToken))
            {
                return false;
            }

            await ScrubBlocksAsync(userId, batchSize, cancellationToken);

            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            await LazyServiceProvider.LazyGetRequiredService<IRepository<AgentPresence, Guid>>()
                .DeleteDirectAsync(p => p.UserId == userId, cancellationToken);
            var request = await _erasureRepository.GetAsync(erasureId, cancellationToken: cancellationToken);
            request.MarkCompleted(Clock.Now);
            await _erasureRepository.UpdateAsync(request, autoSave: true, cancellationToken);
            await uow.CompleteAsync(cancellationToken);
        }

        Logger.LogInformation("The tracked data of deleted account {UserId} was scrubbed.", userId);
        return true;
    }

    /// <summary>The placeholder user name (and address mailbox) of a deleted account.</summary>
    public static string PlaceholderUserName(Guid userId) => $"deleted-{userId:N}";

    /// <summary>A stored timeline without its per-minute application names (the minutes and counts are kept); an
    /// unreadable timeline becomes empty.</summary>
    public static string ScrubTimeline(string? timelineJson)
    {
        if (string.IsNullOrWhiteSpace(timelineJson))
        {
            return "[]";
        }

        try
        {
            if (JsonNode.Parse(timelineJson) is not JsonArray minutes)
            {
                return "[]";
            }

            foreach (var minute in minutes.OfType<JsonObject>())
            {
                foreach (var name in minute.Select(p => p.Key).Where(IsAppNameKey).ToList())
                {
                    minute.Remove(name);
                }
            }

            return minutes.ToJsonString();
        }
        catch (JsonException)
        {
            return "[]";
        }
    }

    private static bool IsAppNameKey(string key) => string.Equals(key, "AppName", StringComparison.OrdinalIgnoreCase);

    private async Task LeaveEverythingAsync(Guid userId, List<WorkspaceMember> memberships)
    {
        foreach (var membership in memberships)
        {
            membership.Remove();
        }

        if (memberships.Count > 0)
        {
            await _workspaceMemberRepository.UpdateManyAsync(memberships, autoSave: true);
        }

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            await LazyServiceProvider.LazyGetRequiredService<IRepository<ProjectMember, Guid>>()
                .DeleteDirectAsync(m => m.UserId == userId);
            await LazyServiceProvider.LazyGetRequiredService<IRepository<AgentPresence, Guid>>()
                .DeleteDirectAsync(p => p.UserId == userId);
        }
    }

    private async Task AnonymizeAsync(IdentityUser user)
    {
        var placeholder = PlaceholderUserName(user.Id);
        (await _userManager.SetUserNameAsync(user, placeholder)).CheckErrors();
        (await _userManager.SetEmailAsync(user, $"{placeholder}@{PlaceholderEmailDomain}")).CheckErrors();
        (await _userManager.SetPhoneNumberAsync(user, null)).CheckErrors();
        user.Name = null;
        user.Surname = null;
        user.SetIsActive(false);
        (await _userManager.UpdateAsync(user)).CheckErrors();
        (await _userManager.DeleteAsync(user)).CheckErrors();
    }

    private async Task<bool> DeleteCapturesAsync(Guid userId, int batchSize, CancellationToken cancellationToken)
    {
        var after = Guid.Empty;
        var anyFailed = false;
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            var cursor = after;
            var screenshots = await _screenshotRepository.GetQueryableAsync();
            var activities = await _activityRepository.GetQueryableAsync();
            var page = await AsyncExecuter.ToListAsync(
                screenshots
                    .Where(s => s.Id.CompareTo(cursor) > 0 && activities.Any(a => a.Id == s.ActivityId && a.UserId == userId))
                    .OrderBy(s => s.Id)
                    .Take(batchSize),
                cancellationToken);
            if (page.Count == 0)
            {
                await uow.CompleteAsync(cancellationToken);
                return !anyFailed;
            }

            anyFailed |= (await _captureEraser.DeleteAsync(page, throwOnFileFailure: false)).FailedFiles > 0;
            after = page[^1].Id;
            await uow.CompleteAsync(cancellationToken);
        }
    }

    private async Task ScrubBlocksAsync(Guid userId, int batchSize, CancellationToken cancellationToken)
    {
        var after = Guid.Empty;
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            var cursor = after;
            var page = await AsyncExecuter.ToListAsync(
                (await _activityRepository.GetQueryableAsync())
                    .Where(a => a.UserId == userId && a.Id.CompareTo(cursor) > 0 &&
                                (a.ActiveWindowsJson != "[]" || a.RunningProgramsJson != "[]" ||
                                 a.Description != null || a.DeviceName != null || a.TimelineJson.Contains("ppName")))
                    .OrderBy(a => a.Id)
                    .Take(batchSize),
                cancellationToken);
            if (page.Count == 0)
            {
                await uow.CompleteAsync(cancellationToken);
                return;
            }

            foreach (var activity in page)
            {
                activity.ActiveWindowsJson = "[]";
                activity.RunningProgramsJson = "[]";
                activity.Description = null;
                activity.DeviceName = null;
                activity.TimelineJson = ScrubTimeline(activity.TimelineJson);
            }

            await _activityRepository.UpdateManyAsync(page, autoSave: true, cancellationToken);
            after = page[^1].Id;
            await uow.CompleteAsync(cancellationToken);
        }
    }
}
