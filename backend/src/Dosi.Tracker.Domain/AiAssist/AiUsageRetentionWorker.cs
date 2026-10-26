using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
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

namespace Dosi.Tracker.AiAssist;

/// <summary>What one run of <see cref="AiUsageRetentionWorker.SweepAsync"/> did.</summary>
public class AiUsageRetentionSweepResult
{
    /// <summary>The sweep did not run because another one held the sweep lock.</summary>
    public bool Skipped { get; init; }

    /// <summary>Ledger rows deleted.</summary>
    public int Deleted { get; set; }
}

/// <summary>
/// Deletes usage-ledger rows (<see cref="AiUsageRecord"/>) older than <c>AI:Usage:RetentionDays</c> (400 by default),
/// across every workspace, once a day. Rows go in batches, each in a short transaction of its own, so a large backlog
/// never holds a long one; sweeps do not overlap (named lock, held across replicas when a distributed lock provider is
/// configured). The consent history is not touched: it lives as long as its workspace.
/// </summary>
public class AiUsageRetentionWorker : AsyncPeriodicBackgroundWorkerBase
{
    /// <summary>Name of the lock that keeps two sweeps from running at the same time.</summary>
    public const string LockName = "Tracker:AiUsageRetention";

    /// <summary>Rows deleted per transaction.</summary>
    public const int BatchSize = 1000;

    public AiUsageRetentionWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromHours(24).TotalMilliseconds;
        Timer.RunOnStart = false;
    }

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return SweepAsync(workerContext.ServiceProvider, StoppingToken);
    }

    /// <summary>One sweep (the body of the daily run).</summary>
    public virtual async Task<AiUsageRetentionSweepResult> SweepAsync(IServiceProvider serviceProvider, CancellationToken cancellationToken = default)
    {
        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null
            ? null
            : await distributedLock.TryAcquireAsync(LockName, cancellationToken: cancellationToken);
        if (distributedLock != null && sweepLock == null)
        {
            Logger.LogInformation("Another AI usage retention sweep is running; this one is skipped.");
            return new AiUsageRetentionSweepResult { Skipped = true };
        }

        var retentionDays = Math.Max(1, serviceProvider.GetRequiredService<IOptions<AiOptions>>().Value.Usage.RetentionDays);
        var cutoff = serviceProvider.GetRequiredService<IClock>().Now.AddDays(-retentionDays);
        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var repository = serviceProvider.GetRequiredService<IRepository<AiUsageRecord, Guid>>();

        var result = new AiUsageRetentionSweepResult();
        using (dataFilter.Disable<IWorkspaceScoped>())
        {
            while (true)
            {
                cancellationToken.ThrowIfCancellationRequested();
                using var uow = uowManager.Begin(requiresNew: true, isTransactional: true);
                var ids = await asyncExecuter.ToListAsync(
                    (await repository.GetQueryableAsync())
                        .Where(r => r.CreationTime < cutoff)
                        .OrderBy(r => r.CreationTime)
                        .Select(r => r.Id)
                        .Take(BatchSize),
                    cancellationToken);
                if (ids.Count > 0)
                {
                    await repository.DeleteDirectAsync(r => ids.Contains(r.Id), cancellationToken);
                }

                await uow.CompleteAsync(cancellationToken);
                result.Deleted += ids.Count;
                if (ids.Count < BatchSize)
                {
                    break;
                }
            }
        }

        if (result.Deleted > 0)
        {
            Logger.LogInformation("Deleted {Count} AI usage ledger rows older than {RetentionDays} days.", result.Deleted, retentionDays);
        }

        return result;
    }
}
