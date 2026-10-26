using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.Logging;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.SettingManagement;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Erasure;

/// <summary>
/// Purges a deleted workspace's data: its captures and their files, its tracked time and the records of deletions, its
/// projects (with tasks, members and rates), presence, device revocations, notifications, the AI usage ledger and the AI
/// consent history, invitations, settings and memberships. Its subscription (canceled at deletion) and its invoices stay
/// as the platform's accounting records; the workspace row stays too, without its name (<see cref="Workspace.MarkPurged"/>).
/// Idempotent and resumable: a file that cannot be deleted keeps its capture, and the workspace counts as purged only once
/// nothing of it is left.
/// </summary>
public class WorkspacePurger : TrackerDomainService
{
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly CaptureEraser _captureEraser;

    public WorkspacePurger(
        IRepository<Workspace, Guid> workspaceRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        CaptureEraser captureEraser)
    {
        _workspaceRepository = workspaceRepository;
        _screenshotRepository = screenshotRepository;
        _captureEraser = captureEraser;
    }

    /// <summary>Purges the deleted workspace <paramref name="workspaceId"/>; true once it is fully purged (false while a
    /// capture file could not be deleted: the next run tries again). A workspace that is not deleted is never touched.</summary>
    public virtual async Task<bool> PurgeAsync(Guid workspaceId, int batchSize, CancellationToken cancellationToken = default)
    {
        using (DataFilter.Disable<ISoftDelete>())
        using (CurrentWorkspace.Change(workspaceId))
        {
            var workspace = await FindDeletedAsync(workspaceId);
            if (workspace == null)
            {
                return false;
            }

            if (workspace.PurgedTime != null)
            {
                return true;
            }

            if (!await PurgeCapturesAsync(workspaceId, Math.Max(1, batchSize), cancellationToken))
            {
                return false;
            }

            await PurgeRowsAsync(workspaceId, cancellationToken);

            using (var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true))
            {
                workspace = await _workspaceRepository.GetAsync(workspaceId, cancellationToken: cancellationToken);
                workspace.MarkPurged(Clock.Now);
                await _workspaceRepository.UpdateAsync(workspace, autoSave: true, cancellationToken);
                await uow.CompleteAsync(cancellationToken);
            }

            Logger.LogInformation("The data of deleted workspace {WorkspaceId} was purged.", workspaceId);
            return true;
        }
    }

    private async Task<Workspace?> FindDeletedAsync(Guid workspaceId)
    {
        using var uow = UnitOfWorkManager.Begin(requiresNew: true);
        var workspace = await _workspaceRepository.FindAsync(workspaceId);
        await uow.CompleteAsync();
        return workspace is { IsDeleted: true } ? workspace : null;
    }

    /// <summary>Deletes the captures page by page (in id order, so a capture whose file could not be deleted is passed
    /// over, not retried in a loop); true when none is left.</summary>
    private async Task<bool> PurgeCapturesAsync(Guid workspaceId, int batchSize, CancellationToken cancellationToken)
    {
        var after = Guid.Empty;
        var anyFailed = false;
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            var cursor = after;
            var page = await AsyncExecuter.ToListAsync(
                (await _screenshotRepository.GetQueryableAsync())
                    .Where(s => s.WorkspaceId == workspaceId && s.Id.CompareTo(cursor) > 0)
                    .OrderBy(s => s.Id)
                    .Take(batchSize),
                cancellationToken);
            if (page.Count == 0)
            {
                await uow.CompleteAsync(cancellationToken);
                return !anyFailed;
            }

            var result = await _captureEraser.DeleteAsync(page, throwOnFileFailure: false);
            anyFailed |= result.FailedFiles > 0;
            after = page[^1].Id;
            await uow.CompleteAsync(cancellationToken);
        }
    }

    /// <summary>Deletes every other row of the workspace (one statement per table; children before their parents).</summary>
    private async Task PurgeRowsAsync(Guid workspaceId, CancellationToken cancellationToken)
    {
        using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
        var key = workspaceId.ToString();

        await Repository<Activity>().DeleteDirectAsync(a => a.WorkspaceId == workspaceId, cancellationToken);
        await Repository<ActivityErasure>().DeleteDirectAsync(e => e.WorkspaceId == workspaceId, cancellationToken);
        await Repository<AgentPresence>().DeleteDirectAsync(p => p.WorkspaceId == workspaceId, cancellationToken);
        await Repository<DeviceRevocation>().DeleteDirectAsync(r => r.WorkspaceId == workspaceId, cancellationToken);
        await Repository<Notification>().DeleteDirectAsync(n => n.WorkspaceId == workspaceId, cancellationToken);
        await Repository<AiUsageRecord>().DeleteDirectAsync(r => r.WorkspaceId == workspaceId, cancellationToken);
        await Repository<AiConsentEvent>().DeleteDirectAsync(e => e.WorkspaceId == workspaceId, cancellationToken);
        await Repository<WorkspaceInvitation>().DeleteDirectAsync(i => i.WorkspaceId == workspaceId, cancellationToken);
        await Repository<ProjectTask>().DeleteDirectAsync(t => t.WorkspaceId == workspaceId, cancellationToken);
        await Repository<ProjectMemberRate>().DeleteDirectAsync(r => r.WorkspaceId == workspaceId, cancellationToken);
        await Repository<ProjectMember>().DeleteDirectAsync(m => m.WorkspaceId == workspaceId, cancellationToken);
        await Repository<Project>().DeleteDirectAsync(p => p.WorkspaceId == workspaceId, cancellationToken);
        await Repository<WorkspaceMember>().DeleteDirectAsync(m => m.WorkspaceId == workspaceId, cancellationToken);

        var settingRepository = LazyServiceProvider.LazyGetRequiredService<ISettingRepository>();
        var settings = await settingRepository.GetListAsync(WorkspaceSettingValueProvider.ProviderName, key, cancellationToken);
        if (settings.Count > 0)
        {
            await settingRepository.DeleteManyAsync(settings, autoSave: true, cancellationToken);
        }

        await uow.CompleteAsync(cancellationToken);
    }

    private IRepository<TEntity> Repository<TEntity>() where TEntity : class, Volo.Abp.Domain.Entities.IEntity =>
        LazyServiceProvider.LazyGetRequiredService<IRepository<TEntity>>();
}
