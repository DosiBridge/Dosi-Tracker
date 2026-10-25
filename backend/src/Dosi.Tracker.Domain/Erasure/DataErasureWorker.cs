using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Erasure;

/// <summary>What one run of <see cref="DataErasureWorker.SweepAsync"/> did.</summary>
public class DataErasureSweepResult
{
    /// <summary>The sweep did not run because another one held the sweep lock.</summary>
    public bool Skipped { get; init; }

    /// <summary>Deleted workspaces whose data is now fully purged.</summary>
    public int WorkspacesPurged { get; set; }

    /// <summary>Deleted accounts whose tracked data is now fully scrubbed.</summary>
    public int AccountsScrubbed { get; set; }

    /// <summary>Workspaces or accounts left unfinished (a file could not be deleted, or an error); the next sweep
    /// continues them.</summary>
    public int Unfinished { get; set; }
}

/// <summary>
/// The erasure sweep (<see cref="DataErasureOptions"/>): purges the data of every deleted workspace whose grace period is
/// over (<see cref="WorkspacePurger"/>), and scrubs the tracked data of every deleted account
/// (<see cref="AccountEraser.ScrubAsync"/>). Runs at start-up, then every <see cref="DataErasureOptions.PeriodMinutes"/>;
/// sweeps do not overlap (named lock, held across replicas when a distributed lock provider is configured), and each
/// workspace or account is handled on its own, so one failure stops nothing else.
/// </summary>
public class DataErasureWorker : AsyncPeriodicBackgroundWorkerBase
{
    /// <summary>Name of the lock that keeps two sweeps from running at the same time.</summary>
    public const string LockName = "Tracker:DataErasureSweep";

    private readonly DataErasureOptions _options;

    public DataErasureWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory,
        IOptions<DataErasureOptions> options)
        : base(timer, serviceScopeFactory)
    {
        _options = options.Value;
        Timer.Period = (int)TimeSpan.FromMinutes(Math.Max(1, _options.PeriodMinutes)).TotalMilliseconds;
        Timer.RunOnStart = true;
    }

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return SweepAsync(workerContext.ServiceProvider, StoppingToken);
    }

    /// <summary>One sweep (the body of the periodic run).</summary>
    public virtual async Task<DataErasureSweepResult> SweepAsync(IServiceProvider serviceProvider, CancellationToken cancellationToken = default)
    {
        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null
            ? null
            : await distributedLock.TryAcquireAsync(LockName, cancellationToken: cancellationToken);
        if (distributedLock != null && sweepLock == null)
        {
            Logger.LogInformation("Another erasure sweep is running; this one is skipped.");
            return new DataErasureSweepResult { Skipped = true };
        }

        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var workspaceRepository = serviceProvider.GetRequiredService<IRepository<Workspace, Guid>>();
        var erasureRepository = serviceProvider.GetRequiredService<IRepository<AccountErasure, Guid>>();
        var purger = serviceProvider.GetRequiredService<WorkspacePurger>();
        var accountEraser = serviceProvider.GetRequiredService<AccountEraser>();
        var purgeBefore = serviceProvider.GetRequiredService<IClock>().Now.AddDays(-Math.Max(0, _options.WorkspaceGraceDays));

        Guid[] dueWorkspaces;
        Guid[] pendingAccounts;
        using (var readUow = uowManager.Begin(requiresNew: true))
        {
            using (dataFilter.Disable<ISoftDelete>())
            {
                dueWorkspaces = (await asyncExecuter.ToListAsync((await workspaceRepository.GetQueryableAsync())
                        .Where(w => w.IsDeleted && w.PurgedTime == null && w.DeletionTime <= purgeBefore)
                        .OrderBy(w => w.DeletionTime)
                        .Select(w => w.Id), cancellationToken))
                    .ToArray();
            }

            pendingAccounts = (await asyncExecuter.ToListAsync((await erasureRepository.GetQueryableAsync())
                    .Where(e => e.CompletedTime == null)
                    .OrderBy(e => e.CreationTime)
                    .Select(e => e.Id), cancellationToken))
                .ToArray();
            await readUow.CompleteAsync(cancellationToken);
        }

        var result = new DataErasureSweepResult();
        foreach (var workspaceId in dueWorkspaces)
        {
            try
            {
                if (await purger.PurgeAsync(workspaceId, _options.BatchSize, cancellationToken))
                {
                    result.WorkspacesPurged++;
                }
                else
                {
                    result.Unfinished++;
                }
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                result.Unfinished++;
                Logger.LogError(ex, "Purging deleted workspace {WorkspaceId} failed; the next sweep continues it.", workspaceId);
            }
        }

        foreach (var erasureId in pendingAccounts)
        {
            try
            {
                if (await accountEraser.ScrubAsync(erasureId, _options.BatchSize, cancellationToken))
                {
                    result.AccountsScrubbed++;
                }
                else
                {
                    result.Unfinished++;
                }
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                result.Unfinished++;
                Logger.LogError(ex, "Scrubbing account erasure {ErasureId} failed; the next sweep continues it.", erasureId);
            }
        }

        return result;
    }
}
