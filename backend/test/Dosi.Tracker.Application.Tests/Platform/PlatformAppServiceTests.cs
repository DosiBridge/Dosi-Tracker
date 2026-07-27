using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.Platform;

public abstract class PlatformAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IPlatformAppService _platform;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;

    protected PlatformAppServiceTests()
    {
        _platform = GetRequiredService<IPlatformAppService>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    [Fact]
    public async Task Overview_Should_Aggregate_Subscriptions_Invoices_And_Plans_Across_Tenants()
    {
        var starter = await WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter"));
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();

        using (_currentTenant.Change(tenantA))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var active = new Subscription(_guidGenerator.Create(), tenantA, starter.Id) { Status = "active" };
                await _subscriptions.InsertAsync(active);
                await _invoices.InsertAsync(new Invoice(
                    _guidGenerator.Create(), tenantA, 12m, new DateTime(2026, 7, 1, 0, 0, 0, DateTimeKind.Utc)));
            });
        }

        using (_currentTenant.Change(tenantB))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                // ctor defaults Status to "trialing"
                await _subscriptions.InsertAsync(new Subscription(_guidGenerator.Create(), tenantB, starter.Id));
            });
        }

        var overview = await _platform.GetOverviewAsync();

        overview.ActiveSubscriptions.ShouldBe(1);
        overview.TrialingSubscriptions.ShouldBe(1);
        overview.Plans.Single(p => p.Name == "Starter").SubscriberCount.ShouldBe(2);
        overview.PendingInvoices.ShouldBe(1);
        overview.TotalInvoiced.ShouldBe(12m);
        overview.TenantCount.ShouldBeGreaterThanOrEqualTo(0);
    }

    [Fact]
    public async Task Plans_Should_Round_Trip_Through_Create_Update_Delete()
    {
        var created = await _platform.CreatePlanAsync(new CreateUpdatePlanDto
        {
            Name = "Enterprise", PricePerUser = 25m, MaxSeats = 500, TrialDays = 30
        });
        created.Id.ShouldNotBe(Guid.Empty);
        created.Name.ShouldBe("Enterprise");

        var updated = await _platform.UpdatePlanAsync(created.Id, new CreateUpdatePlanDto
        {
            Name = "Enterprise+", PricePerUser = 30m, MaxSeats = 1000, TrialDays = 14
        });
        updated.PricePerUser.ShouldBe(30m);
        updated.Name.ShouldBe("Enterprise+");

        var plans = await _platform.GetPlansAsync();
        plans.Items.ShouldContain(p => p.Id == created.Id && p.Name == "Enterprise+");

        await _platform.DeletePlanAsync(created.Id);
        (await _platform.GetPlansAsync()).Items.ShouldNotContain(p => p.Id == created.Id);
    }

    [Fact]
    public async Task Platform_Console_Should_Be_Refused_To_A_Tenant_Scoped_Caller()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _platform.GetOverviewAsync());
            await Should.ThrowAsync<AbpAuthorizationException>(
                () => _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto()));
        }
    }
}
