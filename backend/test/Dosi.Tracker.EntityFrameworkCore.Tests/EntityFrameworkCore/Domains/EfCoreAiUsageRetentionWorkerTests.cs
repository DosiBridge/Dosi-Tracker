using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

/// <summary>The usage-ledger retention sweep deletes rows older than <c>AI:Usage:RetentionDays</c> in every workspace,
/// in batches, and nothing else: newer rows and the consent history stay.</summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiUsageRetentionWorkerTests : TrackerEntityFrameworkCoreTestBase
{
    private readonly IRepository<AiUsageRecord, Guid> _records;
    private readonly IClock _clock;

    public EfCoreAiUsageRetentionWorkerTests()
    {
        _records = GetRequiredService<IRepository<AiUsageRecord, Guid>>();
        _clock = GetRequiredService<IClock>();
    }

    private AiUsageRetentionWorker NewWorker() => new(
        GetRequiredService<AbpAsyncTimer>(),
        GetRequiredService<IServiceScopeFactory>())
    {
        LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>()
    };

    private Task SeedAsync(Guid workspaceId, int count, int daysAgo)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            for (var i = 0; i < count; i++)
            {
                var record = new AiUsageRecord(Guid.NewGuid(), workspaceId, Guid.NewGuid(), AiFeatureIds.AskId, "ask.v1",
                    AiUsageRecord.SelfAudience, AiOptions.ProviderOpenAi, "gpt-5.4-mini", AiUsageOutcomes.Generated);
                ObjectHelper.TrySetProperty(record, r => r.CreationTime, () => _clock.Now.AddDays(-daysAgo).AddMinutes(-i));
                await _records.InsertAsync(record);
            }
        });
    }

    private Task<int> CountAsync(Guid workspaceId) => WithUnitOfWorkAsync(async () =>
    {
        using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
        {
            return await _records.CountAsync(r => r.WorkspaceId == workspaceId);
        }
    });

    [Fact]
    public void The_Worker_Should_Run_Daily_And_Not_At_Start_Up()
    {
        var timer = GetRequiredService<AbpAsyncTimer>();
        _ = new AiUsageRetentionWorker(timer, GetRequiredService<IServiceScopeFactory>());

        timer.Period.ShouldBe((int)TimeSpan.FromHours(24).TotalMilliseconds);
        timer.RunOnStart.ShouldBeFalse();
        AiUsageRetentionWorker.LockName.ShouldBe("Tracker:AiUsageRetention");
    }

    [Fact]
    public async Task A_Sweep_Should_Delete_Rows_Past_Retention_In_Every_Workspace_And_Keep_The_Rest()
    {
        var workspaceA = Guid.NewGuid();
        var workspaceB = Guid.NewGuid();
        var retention = GetRequiredService<IOptions<AiOptions>>().Value.Usage.RetentionDays;
        retention.ShouldBe(400);

        // More expired rows than one batch, so the sweep has to loop.
        await SeedAsync(workspaceA, AiUsageRetentionWorker.BatchSize + 5, retention + 1);
        await SeedAsync(workspaceA, 3, retention - 1);
        await SeedAsync(workspaceB, 2, retention + 30);
        await SeedAsync(workspaceB, 1, 0);
        await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<AiConsentEvent, Guid>>().InsertAsync(
            new AiConsentEvent(Guid.NewGuid(), workspaceA, Guid.NewGuid(), AiConsentEvent.EnabledAction, "1.0", "managers=1")));

        var result = await NewWorker().SweepAsync(ServiceProvider);

        result.Skipped.ShouldBeFalse();
        result.Deleted.ShouldBe(AiUsageRetentionWorker.BatchSize + 5 + 2);
        (await CountAsync(workspaceA)).ShouldBe(3);
        (await CountAsync(workspaceB)).ShouldBe(1);
        (await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await GetRequiredService<IRepository<AiConsentEvent, Guid>>().CountAsync();
            }
        })).ShouldBe(1);

        // Nothing left to do.
        (await NewWorker().SweepAsync(ServiceProvider)).Deleted.ShouldBe(0);
    }

    [Fact]
    public async Task A_Shorter_Configured_Retention_Should_Be_Honoured()
    {
        var workspaceId = Guid.NewGuid();
        GetRequiredService<IOptions<AiOptions>>().Value.Usage.RetentionDays = 30;
        await SeedAsync(workspaceId, 2, 31);
        await SeedAsync(workspaceId, 2, 29);

        (await NewWorker().SweepAsync(ServiceProvider)).Deleted.ShouldBe(2);
        (await CountAsync(workspaceId)).ShouldBe(2);
    }

    [Fact]
    public async Task A_Sweep_Should_Be_Skipped_While_Another_Holds_The_Lock()
    {
        await using var held = await GetRequiredService<Volo.Abp.DistributedLocking.IAbpDistributedLock>()
            .TryAcquireAsync(AiUsageRetentionWorker.LockName);
        held.ShouldNotBeNull();

        (await NewWorker().SweepAsync(ServiceProvider)).Skipped.ShouldBeTrue();
    }
}
