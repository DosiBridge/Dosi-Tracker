using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Deletes tracked time blocks older than a workspace's activity retention window (REQ-DAT-01), for the workspaces whose
/// owner opted in; every other workspace keeps its time. It runs at start-up and then every
/// <see cref="ActivityRetentionOptions.PeriodHours"/>, one instance at a time (sweep lock), and delegates the deletion to
/// <see cref="IActivityRetentionService"/>: blocks inside the agents' upload window (<see cref="ActivityIngestOptions"/>)
/// and blocks that still have captures are kept, so capture retention removes a block's captures first. Archived
/// workspaces are left untouched. Per-workspace cursors carry a sweep past kept blocks (in memory: a restart starts again
/// from the oldest).
/// </summary>
public class ActivityRetentionWorker : AsyncPeriodicBackgroundWorkerBase
{
    public const string LockName = "Tracker:ActivityRetentionSweep";

    private static readonly ConcurrentDictionary<Guid, ActivityRetentionCursor> Cursors = new();

    private readonly IOptions<ActivityRetentionOptions> _options;
    private readonly ILogger<ActivityRetentionWorker> _logger;

    public ActivityRetentionWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory,
        IOptions<ActivityRetentionOptions> options,
        ILogger<ActivityRetentionWorker>? logger = null)
        : base(timer, serviceScopeFactory)
    {
        _options = options;
        _logger = logger ?? NullLogger<ActivityRetentionWorker>.Instance;
        var periodHours = options.Value.PeriodHours > 0 ? options.Value.PeriodHours : 24;
        Timer.Period = (int)TimeSpan.FromHours(periodHours).TotalMilliseconds;
        Timer.RunOnStart = true;
    }

    /// <summary>Forgets where the last sweep of a workspace stopped (its policy changed).</summary>
    public static void ResetCursor(Guid workspaceId) => Cursors.TryRemove(workspaceId, out _);

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return SweepAsync(workerContext.ServiceProvider);
    }

    /// <summary>One sweep over the active workspaces; safe to call directly (tests, administration).</summary>
    public virtual async Task<ActivityRetentionSweepResult> SweepAsync(IServiceProvider serviceProvider)
    {
        var options = _options.Value;
        if (!options.IsEnabled)
        {
            _logger.LogInformation("Activity retention is disabled by configuration; sweep skipped.");
            return new ActivityRetentionSweepResult { Skipped = true, SkipReason = "Disabled by configuration" };
        }

        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null ? null : await distributedLock.TryAcquireAsync(LockName);
        if (distributedLock != null && sweepLock == null)
        {
            _logger.LogInformation("Another activity retention sweep is running; this one is skipped.");
            return new ActivityRetentionSweepResult { Skipped = true, SkipReason = "Lock acquisition failed" };
        }

        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var workspaceSettings = serviceProvider.GetRequiredService<WorkspaceSettingsManager>();
        var retentionService = serviceProvider.GetRequiredService<IActivityRetentionService>();
        var clock = serviceProvider.GetRequiredService<IClock>();
        var ingestionHorizon = serviceProvider.GetRequiredService<IOptions<ActivityIngestOptions>>().Value.MaxBlockAge;
        var batchSize = Math.Clamp(options.BatchSize, 1, ActivityRetentionPolicy.MaxBatchSize);
        var maxPages = Math.Max(1, options.MaxPagesPerWorkspace);

        var result = new ActivityRetentionSweepResult();
        foreach (var workspaceId in await ActiveWorkspaceIdsAsync(serviceProvider, uowManager))
        {
            try
            {
                (bool IsEnabled, int RetentionDays) policy;
                using (var uow = uowManager.Begin(requiresNew: true))
                {
                    policy = await workspaceSettings.GetActivityRetentionPolicyAsync(workspaceId);
                    await uow.CompleteAsync();
                }

                if (!policy.IsEnabled)
                {
                    continue;
                }

                // One cutoff for the whole sweep of a workspace, so its pages never drift.
                var cutoff = clock.Now - TimeSpan.FromDays(policy.RetentionDays);
                var ingestionCutoff = clock.Now - ingestionHorizon;
                Cursors.TryGetValue(workspaceId, out var cursor);

                for (var page = 0; page < maxPages; page++)
                {
                    result.TotalBatchesProcessed++;
                    var run = await retentionService.CleanExpiredActivitiesAsync(new ActivityRetentionPolicy(
                        workspaceId,
                        TimeSpan.FromDays(policy.RetentionDays),
                        ingestionHorizon,
                        batchSize,
                        allowArchivedWorkspace: false,
                        cutoff: cutoff,
                        cursor: cursor,
                        ingestionCutoff: ingestionCutoff));

                    result.TotalDeleted += run.DeletedCount;
                    result.TotalPreserved += run.PreservedReferencedCount + run.PreservedReplaySafetyCount;

                    if (run.Skipped || !run.HasMore || run.NextCursor == null)
                    {
                        // The end of the expired blocks: the next sweep starts again from the oldest.
                        Cursors.TryRemove(workspaceId, out _);
                        break;
                    }

                    cursor = run.NextCursor;
                    Cursors[workspaceId] = cursor;
                }

                result.WorkspacesSwept++;
            }
            catch (Exception exception)
            {
                result.Errors++;
                _logger.LogError(exception,
                    "Failed to clean expired activities of workspace {WorkspaceId}; continuing with the others.", workspaceId);
            }
        }

        if (result.TotalDeleted > 0)
        {
            _logger.LogInformation("Activity retention deleted {Count} expired block(s) in {Workspaces} workspace(s).",
                result.TotalDeleted, result.WorkspacesSwept);
        }

        return result;
    }

    private static async Task<List<Guid>> ActiveWorkspaceIdsAsync(IServiceProvider serviceProvider, IUnitOfWorkManager uowManager)
    {
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var workspaceRepository = serviceProvider.GetRequiredService<IRepository<Workspace, Guid>>();

        using var uow = uowManager.Begin(requiresNew: true);
        using (dataFilter.Disable<IWorkspaceScoped>())
        {
            var workspaces = await workspaceRepository.GetQueryableAsync();
            var ids = await asyncExecuter.ToListAsync(workspaces.Where(w => !w.IsArchived).OrderBy(w => w.Id).Select(w => w.Id));
            await uow.CompleteAsync();
            return ids;
        }
    }
}

/// <summary>Summary of one activity retention sweep.</summary>
public class ActivityRetentionSweepResult
{
    public bool Skipped { get; set; }
    public string? SkipReason { get; set; }
    public int WorkspacesSwept { get; set; }
    public int TotalDeleted { get; set; }

    /// <summary>Expired blocks kept: they still have captures, or lie inside the upload window.</summary>
    public int TotalPreserved { get; set; }

    public int TotalBatchesProcessed { get; set; }
    public int Errors { get; set; }
}
