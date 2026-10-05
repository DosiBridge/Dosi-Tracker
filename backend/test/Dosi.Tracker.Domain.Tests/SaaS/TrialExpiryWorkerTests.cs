using System;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.DependencyInjection;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Volo.Abp.Threading;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class TrialExpiryWorkerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly TrialExpiryWorker _worker;
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly ICurrentTenant _currentTenant;
    private readonly IGuidGenerator _guidGenerator;

    protected TrialExpiryWorkerTests()
    {
        _worker = GetRequiredService<TrialExpiryWorker>();
        _tenantManager = GetRequiredService<ITenantManager>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    /// <summary>A workspace (tenant) with a Starter subscription in the given state.</summary>
    private async Task<Guid> CreateWorkspaceAsync(string name, string status, DateTime? trialEndsAt = null)
    {
        var starter = await WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter"));

        // Tenant and subscription in one unit of work: the default-subscription handler then leaves it alone.
        return await WithUnitOfWorkAsync(async () =>
        {
            var tenant = await _tenantManager.CreateAsync(name);
            await _tenantRepository.InsertAsync(tenant);
            using (_currentTenant.Change(tenant.Id))
            {
                await _subscriptions.InsertAsync(new Subscription(_guidGenerator.Create(), tenant.Id, starter.Id)
                {
                    Status = status,
                    TrialEndsAt = trialEndsAt
                });
            }

            return tenant.Id;
        });
    }

    private async Task InvoiceAsync(Guid tenantId, DateTime dueDate)
    {
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _invoices.InsertAsync(new Invoice(_guidGenerator.Create(), tenantId, 6m, dueDate)));
        }
    }

    private async Task<Subscription> GetSubscriptionAsync(Guid tenantId)
    {
        using (_currentTenant.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(() => _subscriptions.FirstAsync(s => s.TenantId == tenantId));
        }
    }

    [Fact]
    public async Task The_Sweep_Should_Review_Every_Workspace_That_Needs_It()
    {
        var ended = await CreateWorkspaceAsync("ended", Subscription.TrialingStatus, DateTime.UtcNow.AddHours(-1));
        var ending = await CreateWorkspaceAsync("ending", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(2));
        var running = await CreateWorkspaceAsync("running", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(10));
        var overdue = await CreateWorkspaceAsync("overdue", Subscription.ActiveStatus);
        await InvoiceAsync(overdue, DateTime.UtcNow.AddDays(-1));
        var current = await CreateWorkspaceAsync("current", Subscription.ActiveStatus);
        await InvoiceAsync(current, DateTime.UtcNow.AddDays(5));

        var result = await _worker.CheckSubscriptionsAsync(ServiceProvider);

        result.Skipped.ShouldBeFalse();
        result.TrialsEnded.ShouldBe(1);
        result.TrialRemindersSent.ShouldBe(1);
        result.MarkedPastDue.ShouldBe(1);
        result.Failed.ShouldBe(0);

        (await GetSubscriptionAsync(ended)).Status.ShouldBe(Subscription.PastDueStatus);
        (await GetSubscriptionAsync(ending)).Status.ShouldBe(Subscription.TrialingStatus);
        (await GetSubscriptionAsync(running)).Status.ShouldBe(Subscription.TrialingStatus);
        (await GetSubscriptionAsync(overdue)).Status.ShouldBe(Subscription.PastDueStatus);
        (await GetSubscriptionAsync(current)).Status.ShouldBe(Subscription.ActiveStatus);

        // Idempotent: a second sweep has nothing left to do.
        var second = await _worker.CheckSubscriptionsAsync(ServiceProvider);
        second.TrialsEnded.ShouldBe(0);
        second.TrialRemindersSent.ShouldBe(0);
        second.MarkedPastDue.ShouldBe(0);
    }

    [Fact]
    public async Task Subscriptions_Of_Workspaces_That_No_Longer_Exist_Should_Be_Skipped()
    {
        var orphanTenantId = Guid.NewGuid(); // no tenant row: a workspace deleted before cancellation existed
        var starter = await WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter"));
        using (_currentTenant.Change(orphanTenantId))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(
                new Subscription(_guidGenerator.Create(), orphanTenantId, starter.Id) { TrialEndsAt = DateTime.UtcNow.AddDays(-1) }));
        }

        var result = await _worker.CheckSubscriptionsAsync(ServiceProvider);

        result.TrialsEnded.ShouldBe(0);
        (await GetSubscriptionAsync(orphanTenantId)).Status.ShouldBe(Subscription.TrialingStatus);
    }

    [Fact]
    public async Task A_Sweep_Should_Be_Skipped_While_Another_One_Runs()
    {
        var ended = await CreateWorkspaceAsync("ended", Subscription.TrialingStatus, DateTime.UtcNow.AddHours(-1));

        await using (var running = await GetRequiredService<IAbpDistributedLock>().TryAcquireAsync(TrialExpiryWorker.LockName))
        {
            running.ShouldNotBeNull();

            var skipped = await _worker.CheckSubscriptionsAsync(ServiceProvider);

            skipped.Skipped.ShouldBeTrue();
            (await GetSubscriptionAsync(ended)).Status.ShouldBe(Subscription.TrialingStatus);
        }

        (await _worker.CheckSubscriptionsAsync(ServiceProvider)).TrialsEnded.ShouldBe(1);
    }

    [Fact]
    public void The_Worker_Should_Run_At_Start_Up_And_Then_Hourly()
    {
        var probe = new ScheduleProbe(ServiceProvider);

        probe.RunsOnStart.ShouldBeTrue();
        probe.Period.ShouldBe((int)TimeSpan.FromHours(1).TotalMilliseconds);
    }

    [Fact]
    public async Task Each_Scheduled_Run_Should_Sweep()
    {
        var ended = await CreateWorkspaceAsync("ended", Subscription.TrialingStatus, DateTime.UtcNow.AddHours(-1));

        await new ScheduleProbe(ServiceProvider).RunScheduledAsync(ServiceProvider);

        (await GetSubscriptionAsync(ended)).Status.ShouldBe(Subscription.PastDueStatus);
    }

    /// <summary>The worker with its timer settings visible and its timer entry callable.</summary>
    private sealed class ScheduleProbe : TrialExpiryWorker
    {
        public ScheduleProbe(IServiceProvider serviceProvider)
            : base(new AbpAsyncTimer(), serviceProvider.GetRequiredService<IServiceScopeFactory>())
        {
            LazyServiceProvider = serviceProvider.GetRequiredService<IAbpLazyServiceProvider>();
        }

        public bool RunsOnStart => Timer.RunOnStart;

        /// <summary>What the timer does each time it fires.</summary>
        public Task RunScheduledAsync(IServiceProvider serviceProvider)
        {
            return DoWorkAsync(new PeriodicBackgroundWorkerContext(serviceProvider));
        }
    }
}
