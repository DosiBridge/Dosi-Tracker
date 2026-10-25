using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Dosi.Tracker.Workspaces;
using Dosi.Tracker.Settings;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Periodic background worker adapter for capture retention (REQ-DAT-01).
/// Runs by default (CaptureRetention:IsEnabled, see <see cref="CaptureRetentionOptions"/>) but deletes captures only of
/// workspaces whose owner opted in to a retention window. The first sweep runs at start-up and then every
/// <see cref="CaptureRetentionOptions.PeriodHours"/>, so a service restarted daily (deploys, scaling, crashes) still
/// sweeps; the sweep lock keeps concurrent instances from sweeping at once.
/// Executes sweeps across active workspaces under a distributed lock, delegating actual
/// workspace-isolated cleanup to ICaptureRetentionService.
/// Maintains workspace-scoped continuation cursors across sweeps to ensure forward progress
/// past preserved poison/failing rows, with bounded pages per sweep and deterministic wrap-around. The cursors live in
/// memory: after a restart a sweep starts again from a workspace's oldest expired capture.
/// </summary>
public class CaptureRetentionWorker : AsyncPeriodicBackgroundWorkerBase
{
    public const string LockName = "Tracker:CaptureRetentionSweep";

    private readonly IOptions<CaptureRetentionOptions> _options;
    private readonly ILogger<CaptureRetentionWorker> _logger;
    private static readonly ConcurrentDictionary<Guid, CaptureRetentionCursor> _sharedCursors = new();
    private readonly ConcurrentDictionary<Guid, CaptureRetentionCursor> _workspaceCursors = _sharedCursors;

    /// <summary>
    /// Gets the current continuation cursor for the specified workspace, if any.
    /// </summary>
    public CaptureRetentionCursor? GetWorkspaceCursor(Guid workspaceId) =>
        _workspaceCursors.TryGetValue(workspaceId, out var cursor) ? cursor : null;

    /// <summary>
    /// Explicitly resets or clears the continuation cursor for the specified workspace.
    /// </summary>
    public void ResetWorkspaceCursor(Guid workspaceId) =>
        _workspaceCursors.TryRemove(workspaceId, out _);

    /// <summary>
    /// Static helper to reset the continuation cursor for the specified workspace when policy changes.
    /// </summary>
    public static void ResetCursor(Guid workspaceId) =>
        _sharedCursors.TryRemove(workspaceId, out _);

    public CaptureRetentionWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory,
        IOptions<CaptureRetentionOptions> options,
        ILogger<CaptureRetentionWorker>? logger = null)
        : base(timer, serviceScopeFactory)
    {
        _options = options;
        _logger = logger ?? Microsoft.Extensions.Logging.Abstractions.NullLogger<CaptureRetentionWorker>.Instance;
        var periodHours = options.Value.PeriodHours > 0 ? options.Value.PeriodHours : 24;
        Timer.Period = (int)TimeSpan.FromHours(periodHours).TotalMilliseconds;
        Timer.RunOnStart = true; // A restart must never postpone the sweep by a whole period.
    }

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return SweepAsync(workerContext.ServiceProvider);
    }

    /// <summary>
    /// Sweeps active workspaces and performs bounded age-based capture retention.
    /// Safe to call directly in tests or administrative operations.
    /// </summary>
    public virtual async Task<CaptureRetentionSweepResult> SweepAsync(IServiceProvider serviceProvider)
    {
        if (!_options.Value.IsEnabled)
        {
            _logger.LogInformation("Capture retention worker is disabled by configuration; sweep skipped.");
            return new CaptureRetentionSweepResult
            {
                Skipped = true,
                SkipReason = "Disabled by configuration"
            };
        }

        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null ? null : await distributedLock.TryAcquireAsync(LockName);
        if (distributedLock != null && sweepLock == null)
        {
            _logger.LogInformation("Another capture retention sweep is running; this one is skipped.");
            return new CaptureRetentionSweepResult
            {
                Skipped = true,
                SkipReason = "Lock acquisition failed"
            };
        }

        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var workspaceRepository = serviceProvider.GetRequiredService<IRepository<Workspace, Guid>>();
        var retentionService = serviceProvider.GetRequiredService<ICaptureRetentionService>();

        List<Guid> workspaceIds;
        using (var readUow = uowManager.Begin(requiresNew: true))
        using (dataFilter.Disable<IWorkspaceScoped>())
        {
            var workspaces = await workspaceRepository.GetQueryableAsync();
            // In periodic sweep, only active (non-archived) workspaces are swept by default
            workspaceIds = await asyncExecuter.ToListAsync(
                workspaces.Where(w => !w.IsArchived).Select(w => w.Id));
            await readUow.CompleteAsync();
        }

        var clock = serviceProvider.GetRequiredService<IClock>();
        var workspaceSettings = serviceProvider.GetRequiredService<WorkspaceSettingsManager>();
        var sweepResult = new CaptureRetentionSweepResult();
        var batchSize = Math.Clamp(_options.Value.BatchSize, 1, CaptureRetentionPolicy.MaxBatchSize);
        var maxPages = Math.Max(1, _options.Value.MaxPagesPerWorkspace);

        foreach (var workspaceId in workspaceIds)
        {
            try
            {
                // REQ-DAT-01: Workspace-specific opt-in and retention policy.
                // Explicit opt-in defaults to FALSE. Unset or malformed policy fails closed (skipped).
                (bool isEnabled, int retentionDays) wsPolicy;
                using (var uow = uowManager.Begin(requiresNew: true))
                {
                    wsPolicy = await workspaceSettings.GetCaptureRetentionPolicyAsync(workspaceId);
                    await uow.CompleteAsync();
                }

                if (!wsPolicy.isEnabled)
                {
                    continue; // Skip opted-out or unset workspaces!
                }

                if (wsPolicy.retentionDays < 1 || wsPolicy.retentionDays > 730)
                {
                    _logger.LogWarning("Workspace {WorkspaceId} has invalid retention days {Days}; failing closed.",
                        workspaceId, wsPolicy.retentionDays);
                    continue; // Malformed policy fails closed!
                }

                var retentionPeriod = TimeSpan.FromDays(wsPolicy.retentionDays);

                // Freeze deterministic cutoff timestamp for this workspace sweep so pagination does not drift across pages
                var cutoff = clock.Now - retentionPeriod;
                _workspaceCursors.TryGetValue(workspaceId, out var cursor);
                var pagesProcessed = 0;

                while (pagesProcessed < maxPages)
                {
                    pagesProcessed++;
                    sweepResult.TotalBatchesProcessed++;

                    var policy = new CaptureRetentionPolicy(
                        workspaceId: workspaceId,
                        retentionPeriod: retentionPeriod,
                        batchSize: batchSize,
                        kinds: null,
                        allowArchivedWorkspace: false,
                        cutoff: cutoff,
                        cursor: cursor);

                    var runResult = await retentionService.CleanExpiredCapturesAsync(policy);

                    if (runResult.Skipped)
                    {
                        break;
                    }

                    sweepResult.TotalDeleted += runResult.DeletedCount;
                    sweepResult.TotalFailedBlobs += runResult.FailedBlobCount;
                    sweepResult.TotalMissingBlobs += runResult.MissingBlobCount;
                    sweepResult.TotalCorruptKeys += runResult.CorruptKeyCount;

                    if (runResult.HasMore && runResult.NextCursor != null)
                    {
                        cursor = runResult.NextCursor;
                        _workspaceCursors[workspaceId] = cursor;
                    }
                    else
                    {
                        // Reached end of candidate stream: wrap around to beginning for next sweep
                        _workspaceCursors.TryRemove(workspaceId, out _);
                        cursor = null;
                        break;
                    }
                }

                sweepResult.WorkspacesSwept++;
            }
            catch (Exception ex)
            {
                sweepResult.Errors++;
                _logger.LogError(ex,
                    "Failed to clean expired captures for workspace {WorkspaceId}; continuing with remaining workspaces.",
                    workspaceId);
            }
        }

        return sweepResult;
    }
}
