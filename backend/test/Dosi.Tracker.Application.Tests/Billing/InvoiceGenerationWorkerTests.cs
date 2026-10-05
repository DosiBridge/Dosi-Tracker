using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Volo.Abp.Threading;
using Xunit;

namespace Dosi.Tracker.Billing;

public abstract class InvoiceGenerationWorkerTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;
    private readonly IDataFilter _dataFilter;
    private readonly InvoiceGenerationWorker _worker;

    protected InvoiceGenerationWorkerTests()
    {
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _tenantManager = GetRequiredService<ITenantManager>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _dataFilter = GetRequiredService<IDataFilter>();
        _worker = GetRequiredService<InvoiceGenerationWorker>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private Task<Plan> StarterAsync()
    {
        return WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter")); // $6/user
    }

    /// <summary>A workspace (tenant) subscribed to <paramref name="planId"/>; both saved together so the
    /// default-subscription handler leaves the subscription as given.</summary>
    private Task<Guid> CreateWorkspaceAsync(string name, Guid planId, string status)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var tenant = await _tenantManager.CreateAsync(name);
            await _tenantRepository.InsertAsync(tenant);
            using (_currentTenant.Change(tenant.Id))
            {
                await _subscriptions.InsertAsync(
                    new Subscription(_guidGenerator.Create(), tenant.Id, planId) { Status = status });
            }

            return tenant.Id;
        });
    }

    private Task<List<Invoice>> AllInvoicesAsync()
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (_dataFilter.Disable<IMultiTenant>())
            {
                return await _invoices.GetListAsync();
            }
        });
    }

    /// <summary>The worker's services, with <see cref="InvoiceGenerator"/> replaced by a scripted one.</summary>
    private IServiceProvider WithGenerator(Action<ScriptedInvoiceGenerator> script)
    {
        var generator = ActivatorUtilities.CreateInstance<ScriptedInvoiceGenerator>(ServiceProvider);
        generator.LazyServiceProvider = ServiceProvider.GetRequiredService<IAbpLazyServiceProvider>();
        script(generator);
        return new GeneratorOverridingServiceProvider(ServiceProvider, generator);
    }

    [Fact]
    public async Task Sweep_Should_Invoice_Active_Subscriptions_But_Never_Trialing_Ones()
    {
        var starter = await StarterAsync();
        var activeTenant = await CreateWorkspaceAsync("active", starter.Id, Subscription.ActiveStatus);
        await CreateWorkspaceAsync("trialing", starter.Id, Subscription.TrialingStatus);
        await CreateWorkspaceAsync("pastdue", starter.Id, Subscription.PastDueStatus);

        var result = await _worker.GenerateInvoicesAsync(ServiceProvider);

        result.Generated.ShouldBe(1);
        result.Failed.ShouldBe(0);
        var invoices = await AllInvoicesAsync();
        var invoice = invoices.ShouldHaveSingleItem();
        invoice.TenantId.ShouldBe(activeTenant);
        invoice.Amount.ShouldBe(6m); // one seat minimum
        invoice.BillingPeriodStart.ShouldBe(Invoice.GetBillingPeriodStart(DateTime.UtcNow));

        // A second sweep in the same month bills nobody again.
        (await _worker.GenerateInvoicesAsync(ServiceProvider)).Generated.ShouldBe(0);
        (await AllInvoicesAsync()).Count.ShouldBe(1);
    }

    [Fact]
    public async Task Deleted_Workspaces_Should_Never_Be_Invoiced()
    {
        var starter = await StarterAsync();
        var deleted = await CreateWorkspaceAsync("deleted", starter.Id, Subscription.ActiveStatus);
        await GetRequiredService<ITenantAppService>().DeleteAsync(deleted);

        // A subscription whose workspace row is gone altogether (deleted before deletions canceled subscriptions).
        var orphan = Guid.NewGuid();
        using (_currentTenant.Change(orphan))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(
                new Subscription(_guidGenerator.Create(), orphan, starter.Id) { Status = Subscription.ActiveStatus }));
        }

        var result = await _worker.GenerateInvoicesAsync(ServiceProvider);

        result.Generated.ShouldBe(0);
        (await AllInvoicesAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Sweep_That_Loses_The_Race_For_A_Period_Should_Count_It_As_Invoiced()
    {
        var starter = await StarterAsync();
        var tenantId = await CreateWorkspaceAsync("raced", starter.Id, Subscription.ActiveStatus);
        var period = Invoice.GetBillingPeriodStart(DateTime.UtcNow);

        // Another replica's sweep inserts this period's invoice right after our check found none.
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _invoices.InsertAsync(
                new Invoice(_guidGenerator.Create(), tenantId, 6m, DateTime.UtcNow.AddDays(14)) { BillingPeriodStart = period }));
        }

        var result = await _worker.GenerateInvoicesAsync(WithGenerator(g => g.MissFirstCheck = true));

        result.AlreadyInvoiced.ShouldBe(1);
        result.Generated.ShouldBe(0);
        result.Failed.ShouldBe(0);
        (await AllInvoicesAsync()).ShouldHaveSingleItem().BillingPeriodStart.ShouldBe(period);
    }

    [Fact]
    public async Task A_Failure_For_One_Workspace_Should_Not_Stop_The_Others()
    {
        var starter = await StarterAsync();
        var failing = await CreateWorkspaceAsync("failing", starter.Id, Subscription.ActiveStatus);
        var healthy = await CreateWorkspaceAsync("healthy", starter.Id, Subscription.ActiveStatus);

        var result = await _worker.GenerateInvoicesAsync(WithGenerator(g => g.FailFor = failing));

        result.Failed.ShouldBe(1);
        result.Generated.ShouldBe(1);
        (await AllInvoicesAsync()).ShouldHaveSingleItem().TenantId.ShouldBe(healthy);
    }

    [Fact]
    public async Task A_Sweep_Should_Be_Skipped_While_Another_One_Runs()
    {
        var starter = await StarterAsync();
        await CreateWorkspaceAsync("waiting", starter.Id, Subscription.ActiveStatus);

        await using (var running = await GetRequiredService<IAbpDistributedLock>().TryAcquireAsync(InvoiceGenerationWorker.LockName))
        {
            running.ShouldNotBeNull();
            (await _worker.GenerateInvoicesAsync(ServiceProvider)).Skipped.ShouldBeTrue();
            (await AllInvoicesAsync()).ShouldBeEmpty();
        }

        (await _worker.GenerateInvoicesAsync(ServiceProvider)).Generated.ShouldBe(1);
    }

    [Fact]
    public void The_Worker_Should_Run_At_Start_Up_And_Then_Hourly()
    {
        // Restarting more often than the period must not postpone billing.
        var probe = new ScheduleProbe(ServiceProvider);

        probe.RunsOnStart.ShouldBeTrue();
        probe.Period.ShouldBe((int)TimeSpan.FromHours(1).TotalMilliseconds);
    }

    [Fact]
    public async Task Each_Scheduled_Run_Should_Sweep()
    {
        var starter = await StarterAsync();
        var tenantId = await CreateWorkspaceAsync("scheduled", starter.Id, Subscription.ActiveStatus);

        await new ScheduleProbe(ServiceProvider).RunScheduledAsync(ServiceProvider);

        (await AllInvoicesAsync()).ShouldHaveSingleItem().TenantId.ShouldBe(tenantId);
    }

    /// <summary>An <see cref="InvoiceGenerator"/> that can miss its first "already invoiced?" check (as when another
    /// sweep's insert is not visible yet) or fail for one workspace.</summary>
    public class ScriptedInvoiceGenerator : InvoiceGenerator
    {
        public ScriptedInvoiceGenerator(
            IRepository<Invoice, Guid> invoiceRepository,
            IRepository<Plan, Guid> planRepository,
            SeatCounter seatCounter)
            : base(invoiceRepository, planRepository, seatCounter)
        {
        }

        public bool MissFirstCheck { get; set; }

        public Guid? FailFor { get; set; }

        public override Task<Invoice?> GenerateForPeriodAsync(Subscription subscription, DateTime periodStart)
        {
            if (FailFor.HasValue && subscription.TenantId == FailFor)
            {
                throw new InvalidOperationException("The billing store is unavailable.");
            }

            return base.GenerateForPeriodAsync(subscription, periodStart);
        }

        public override Task<bool> IsInvoicedAsync(Guid? tenantId, DateTime periodStart)
        {
            if (MissFirstCheck)
            {
                MissFirstCheck = false;
                return Task.FromResult(false);
            }

            return base.IsInvoicedAsync(tenantId, periodStart);
        }
    }

    private sealed class GeneratorOverridingServiceProvider : IServiceProvider
    {
        private readonly IServiceProvider _inner;
        private readonly InvoiceGenerator _generator;

        public GeneratorOverridingServiceProvider(IServiceProvider inner, InvoiceGenerator generator)
        {
            _inner = inner;
            _generator = generator;
        }

        public object? GetService(Type serviceType)
        {
            return serviceType == typeof(InvoiceGenerator) ? _generator : _inner.GetService(serviceType);
        }
    }

    /// <summary>The worker with its timer settings visible and its timer entry callable.</summary>
    private sealed class ScheduleProbe : InvoiceGenerationWorker
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
