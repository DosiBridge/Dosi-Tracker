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
        // No project members seeded -> the paying subscription bills the minimum one seat.
        // Starter is $6/user; only the ACTIVE subscription is revenue (the trial pays nothing) => $6 MRR, 1 seat.
        overview.PaidSeats.ShouldBe(1);
        overview.Mrr.ShouldBe(6m);
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
    public async Task Users_Should_List_Accounts_Across_Tenants_With_Their_Workspace()
    {
        // Registering a workspace provisions a tenant with its own admin user.
        await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "acme",
            AdminEmail = "owner@acme.test",
            AdminPassword = "1q2w3E*acme",
            PlanName = "Free"
        });

        var page = await _platform.GetUsersAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 100 });

        page.TotalCount.ShouldBeGreaterThan(0);
        page.Items.ShouldContain(u => u.Email == "owner@acme.test" && u.TenantName == "acme");
    }

    [Fact]
    public async Task DeletePlan_Should_Refuse_While_Any_Tenant_Subscribes_To_It()
    {
        var plan = await _platform.CreatePlanAsync(new CreateUpdatePlanDto
        {
            Name = "Legacy", PricePerUser = 3m, MaxSeats = 10, TrialDays = 0
        });
        var tenantId = Guid.NewGuid();
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(
                new Subscription(_guidGenerator.Create(), tenantId, plan.Id)));
        }

        var exception = await Should.ThrowAsync<Volo.Abp.BusinessException>(() => _platform.DeletePlanAsync(plan.Id));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanInUse);
        (await _platform.GetPlansAsync()).Items.ShouldContain(p => p.Id == plan.Id);
    }

    [Fact]
    public async Task Invoices_Should_Be_Paged_Newest_Due_Date_First_Across_Tenants()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();
        foreach (var (tenant, month) in new[] { (tenantA, 3), (tenantB, 5), (tenantA, 4) })
        {
            using (_currentTenant.Change(tenant))
            {
                await WithUnitOfWorkAsync(() => _invoices.InsertAsync(new Invoice(
                    _guidGenerator.Create(), tenant, month, new DateTime(2026, month, 1, 0, 0, 0, DateTimeKind.Utc))));
            }
        }

        var first = await _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto { SkipCount = 0, MaxResultCount = 2 });
        first.TotalCount.ShouldBe(3);
        first.Items.Select(i => i.DueDate.Month).ShouldBe(new[] { 5, 4 });
        first.Items[0].TenantId.ShouldBe(tenantB);

        var second = await _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto { SkipCount = 2, MaxResultCount = 2 });
        second.TotalCount.ShouldBe(3);
        second.Items.ShouldHaveSingleItem().DueDate.Month.ShouldBe(3);
        second.Items[0].TenantName.ShouldBe("Host"); // tenant id without a Tenant row resolves to the fallback
    }

    [Fact]
    public async Task Tenant_Created_From_The_Host_Console_Gets_A_Free_Subscription_And_A_Manager_Role()
    {
        var tenant = await GetRequiredService<Volo.Abp.TenantManagement.ITenantAppService>().CreateAsync(
            new Volo.Abp.TenantManagement.TenantCreateDto
            {
                Name = "consoleco",
                AdminEmailAddress = "owner@consoleco.test",
                AdminPassword = "1q2w3E*console"
            });

        using (_currentTenant.Change(tenant.Id))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var subscriptions = await _subscriptions.GetListAsync(s => s.TenantId == tenant.Id);
                var subscription = subscriptions.ShouldHaveSingleItem();
                var free = await _plans.FirstAsync(p => p.Name == Plan.FreePlanName);
                subscription.PlanId.ShouldBe(free.Id);
                subscription.Status.ShouldBe("active");

                var roles = await GetRequiredService<Volo.Abp.Identity.IIdentityRoleRepository>().GetListAsync();
                roles.ShouldContain(r => r.Name == Dosi.Tracker.Identity.TrackerRoles.Manager && r.TenantId == tenant.Id);
            });
        }
    }

    [Fact]
    public async Task Workspace_Sign_Up_Keeps_Its_Own_Plan_And_Gets_A_Manager_Role()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "starterco",
            AdminEmail = "owner@starterco.test",
            AdminPassword = "1q2w3E*starter",
            PlanName = "Starter"
        });

        using (_currentTenant.Change(result.TenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                // The tenant-created handler must not add a second (Free) subscription.
                var subscription = (await _subscriptions.GetListAsync(s => s.TenantId == result.TenantId)).ShouldHaveSingleItem();
                subscription.Status.ShouldBe("trialing");
                (await _plans.GetAsync(subscription.PlanId)).Name.ShouldBe("Starter");

                var roles = await GetRequiredService<Volo.Abp.Identity.IIdentityRoleRepository>().GetListAsync();
                roles.ShouldContain(r => r.Name == Dosi.Tracker.Identity.TrackerRoles.Manager);
            });
        }
    }

    [Fact]
    public async Task Platform_Permission_Should_Be_Host_Only_And_Manager_Permissions_Defined()
    {
        var definitions = GetRequiredService<Volo.Abp.Authorization.Permissions.IPermissionDefinitionManager>();

        var platform = await definitions.GetAsync(Dosi.Tracker.Permissions.TrackerPermissions.Platform.Default);
        platform.MultiTenancySide.ShouldBe(MultiTenancySides.Host);

        foreach (var name in Dosi.Tracker.Identity.TrackerRoles.ManagerPermissions)
        {
            (await definitions.GetOrNullAsync(name)).ShouldNotBeNull(name);
        }
    }

    [Fact]
    public async Task Platform_Console_Should_Be_Refused_To_A_Tenant_Scoped_Caller()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _platform.GetOverviewAsync());
            await Should.ThrowAsync<AbpAuthorizationException>(
                () => _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto()));
            await Should.ThrowAsync<AbpAuthorizationException>(
                () => _platform.GetUsersAsync(new PagedAndSortedResultRequestDto()));
        }
    }
}
