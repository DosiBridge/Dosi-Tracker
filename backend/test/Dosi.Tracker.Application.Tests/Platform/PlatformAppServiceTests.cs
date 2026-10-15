using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Validation;
using System.Collections.Generic;
using Xunit;

namespace Dosi.Tracker.Platform;

public abstract class PlatformAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IPlatformAppService _platform;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly IRepository<Project, Guid> _projects;
    private readonly IRepository<ProjectMember, Guid> _members;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    protected PlatformAppServiceTests()
    {
        _platform = GetRequiredService<IPlatformAppService>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _projects = GetRequiredService<IRepository<Project, Guid>>();
        _members = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private Task<Plan> PlanAsync(string name)
    {
        return WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == name));
    }

    /// <summary>A workspace on the given plan.</summary>
    private async Task<Guid> CreateWorkspaceAsync(string name, string planName, string status, DateTime? trialEndsAt = null)
    {
        var plan = await PlanAsync(planName);
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync(name);
        using (_currentWorkspace.Change(workspaceId))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(new Subscription(_guidGenerator.Create(), workspaceId, plan.Id)
            {
                Status = status,
                TrialEndsAt = trialEndsAt
            }));
        }

        return workspaceId;
    }

    /// <summary>A new project of the workspace with <paramref name="count"/> new members, each an active member of the
    /// workspace (what a seat is); returns the project id.</summary>
    private async Task<Guid> AddSeatsAsync(Guid tenantId, int count)
    {
        var userIds = new List<Guid>();
        Guid projectId;
        using (_currentWorkspace.Change(tenantId))
        {
            projectId = await WithUnitOfWorkAsync(async () =>
            {
                var project = await _projects.InsertAsync(new Project(_guidGenerator.Create(), tenantId, "Work"));
                for (var i = 0; i < count; i++)
                {
                    var userName = $"member-{Guid.NewGuid():N}";
                    var user = new IdentityUser(_guidGenerator.Create(), userName, $"{userName}@seats.test");
                    (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
                    await _members.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, project.Id, user.Id));
                    userIds.Add(user.Id);
                }

                return project.Id;
            });
        }

        foreach (var userId in userIds)
        {
            await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Member, tenantId);
        }

        return projectId;
    }

    private async Task<Invoice> InvoiceAsync(Guid tenantId, decimal amount, DateTime dueDate, DateTime? billingPeriodStart = null)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(() => _invoices.InsertAsync(
                new Invoice(_guidGenerator.Create(), tenantId, amount, dueDate) { BillingPeriodStart = billingPeriodStart }));
        }
    }

    private async Task<Subscription> SubscriptionOfAsync(Guid tenantId)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(() => _subscriptions.FirstAsync(s => s.WorkspaceId == tenantId));
        }
    }

    // ---- Overview ----

    [Fact]
    public async Task Overview_Should_Aggregate_Subscriptions_Invoices_And_Plans_Across_Workspaces()
    {
        var tenantA = await CreateWorkspaceAsync("alpha", "Starter", Subscription.ActiveStatus);
        await CreateWorkspaceAsync("beta", "Starter", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(7));
        await InvoiceAsync(tenantA, 12m, new DateTime(2026, 7, 1, 0, 0, 0, DateTimeKind.Utc));

        var overview = await _platform.GetOverviewAsync();

        overview.WorkspaceCount.ShouldBe(3); // with the seeded test workspace
        overview.ActiveSubscriptions.ShouldBe(1);
        overview.TrialingSubscriptions.ShouldBe(1);
        overview.Plans.Single(p => p.Name == "Starter").SubscriberCount.ShouldBe(2);
        overview.PendingInvoices.ShouldBe(1);
        overview.TotalInvoiced.ShouldBe(12m);
        // No project members seeded -> the paying subscription bills the minimum one seat.
        // Starter is $6/user; only the ACTIVE subscription is revenue (the trial pays nothing) => $6 MRR, 1 seat.
        overview.PaidSeats.ShouldBe(1);
        overview.Mrr.ShouldBe(6m);
    }

    [Fact]
    public async Task Overview_Paid_Seats_Should_Not_Count_Seats_On_Free_Plans()
    {
        var free = await CreateWorkspaceAsync("freeco", Plan.FreePlanName, Subscription.ActiveStatus);
        var paying = await CreateWorkspaceAsync("payco", "Starter", Subscription.ActiveStatus);
        await AddSeatsAsync(free, 3);
        await AddSeatsAsync(paying, 2);

        var overview = await _platform.GetOverviewAsync();

        overview.PaidSeats.ShouldBe(2);
        overview.Mrr.ShouldBe(12m);
        overview.ActiveSubscriptions.ShouldBe(2);
    }

    [Fact]
    public async Task Overview_Should_Ignore_Subscriptions_Without_A_Workspace()
    {
        var orphanTenant = Guid.NewGuid();
        var starter = await PlanAsync("Starter");
        using (_currentWorkspace.Change(orphanTenant))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.InsertAsync(
                new Subscription(_guidGenerator.Create(), orphanTenant, starter.Id) { Status = Subscription.ActiveStatus }));
        }

        var overview = await _platform.GetOverviewAsync();

        overview.ActiveSubscriptions.ShouldBe(0);
        overview.Mrr.ShouldBe(0m);
        overview.Plans.Single(p => p.Name == "Starter").SubscriberCount.ShouldBe(0);
    }

    [Fact]
    public async Task Overview_Should_Not_Count_Memberships_Of_People_Outside_The_Workspace()
    {
        var paying = await CreateWorkspaceAsync("payco", "Starter", Subscription.ActiveStatus);
        await AddSeatsAsync(paying, 2);
        await WithUnitOfWorkAsync(async () =>
        {
            var hostProject = await _projects.InsertAsync(new Project(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, "Host trial"));
            await _members.InsertAsync(new ProjectMember(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, hostProject.Id, Guid.NewGuid()));
        });

        var overview = await _platform.GetOverviewAsync();

        overview.PaidSeats.ShouldBe(2);
        overview.Mrr.ShouldBe(12m);
    }

    [Fact]
    public async Task Overview_Seats_Should_Leave_Out_Deleted_Projects()
    {
        var paying = await CreateWorkspaceAsync("payco", "Starter", Subscription.ActiveStatus);
        await AddSeatsAsync(paying, 2);
        var deletedProject = await AddSeatsAsync(paying, 3);
        using (_currentWorkspace.Change(paying))
        {
            await WithUnitOfWorkAsync(() => _projects.DeleteAsync(deletedProject));
        }

        (await _platform.GetOverviewAsync()).PaidSeats.ShouldBe(2);
    }

    // ---- Plans ----

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
    public async Task Plan_Names_Should_Be_Unique_Ignoring_Case()
    {
        var create = await Should.ThrowAsync<AbpValidationException>(() => _platform.CreatePlanAsync(new CreateUpdatePlanDto
        {
            Name = " starter ", PricePerUser = 7m, MaxSeats = 10, TrialDays = 0
        }));
        create.ValidationErrors.ShouldHaveSingleItem().MemberNames.ShouldContain(nameof(CreateUpdatePlanDto.Name));

        var pro = await _platform.CreatePlanAsync(new CreateUpdatePlanDto { Name = "  Pro  ", PricePerUser = 9m, MaxSeats = 50, TrialDays = 7 });
        pro.Name.ShouldBe("Pro");

        await Should.ThrowAsync<AbpValidationException>(() => _platform.UpdatePlanAsync(pro.Id, new CreateUpdatePlanDto
        {
            Name = "BUSINESS", PricePerUser = 9m, MaxSeats = 50, TrialDays = 7
        }));

        // Keeping its own name (in another case) is not a clash.
        (await _platform.UpdatePlanAsync(pro.Id, new CreateUpdatePlanDto { Name = "PRO", PricePerUser = 10m, MaxSeats = 50, TrialDays = 7 }))
            .Name.ShouldBe("PRO");
    }

    [Fact]
    public async Task DeletePlan_Should_Refuse_While_Any_Workspace_Subscribes_To_It()
    {
        var plan = await _platform.CreatePlanAsync(new CreateUpdatePlanDto
        {
            Name = "Legacy", PricePerUser = 3m, MaxSeats = 10, TrialDays = 0
        });
        await CreateWorkspaceAsync("legacyco", "Legacy", Subscription.ActiveStatus);

        var exception = await Should.ThrowAsync<BusinessException>(() => _platform.DeletePlanAsync(plan.Id));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanInUse);
        (await _platform.GetPlansAsync()).Items.ShouldContain(p => p.Id == plan.Id);
    }

    // ---- Invoices and users ----

    [Fact]
    public async Task Invoices_Should_Be_Paged_Newest_Due_Date_First_Across_Workspaces()
    {
        var tenantA = await CreateWorkspaceAsync("alpha", "Starter", Subscription.ActiveStatus);
        var goneTenant = Guid.NewGuid(); // a workspace that no longer exists
        await InvoiceAsync(tenantA, 3, new DateTime(2026, 3, 1, 0, 0, 0, DateTimeKind.Utc));
        await InvoiceAsync(goneTenant, 5, new DateTime(2026, 5, 1, 0, 0, 0, DateTimeKind.Utc));
        await InvoiceAsync(tenantA, 4, new DateTime(2026, 4, 1, 0, 0, 0, DateTimeKind.Utc), new DateTime(2026, 4, 1, 0, 0, 0, DateTimeKind.Utc));
        await InvoiceAsync(tenantA, 2, new DateTime(2026, 2, 1, 0, 0, 0, DateTimeKind.Utc));

        var first = await _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto { SkipCount = 0, MaxResultCount = 2 });
        first.TotalCount.ShouldBe(4);
        first.Items.Select(i => i.DueDate.Month).ShouldBe(new[] { 5, 4 });
        first.Items[0].WorkspaceId.ShouldBe(goneTenant);
        first.Items[0].WorkspaceName.ShouldBe(PlatformAppService.DeletedWorkspaceName);
        first.Items[0].BillingPeriodStart.ShouldBeNull();
        first.Items[1].WorkspaceName.ShouldBe("alpha");
        first.Items[1].BillingPeriodStart.ShouldBe(new DateTime(2026, 4, 1, 0, 0, 0, DateTimeKind.Utc));

        var second = await _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto { SkipCount = 2, MaxResultCount = 2 });
        second.TotalCount.ShouldBe(4);
        second.Items.Select(i => i.DueDate.Month).ShouldBe(new[] { 3, 2 });
        second.Items[1].WorkspaceName.ShouldBe("alpha");
    }

    [Fact]
    public async Task Users_Should_List_Global_Accounts_With_Their_Workspaces()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            WorkspaceName = "acme",
            Email = "owner@acme.test",
            Password = "1q2w3E*acme",
            PlanName = "Free"
        });
        var userId = await WithUnitOfWorkAsync(async () => (await _userManager.FindByEmailAsync("owner@acme.test"))!.Id);
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Member);

        var page = await _platform.GetUsersAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 100 });

        page.TotalCount.ShouldBeGreaterThan(0);
        var owner = page.Items.Single(u => u.Email == "owner@acme.test");
        owner.WorkspaceNames.ShouldBe(new[] { "acme", TrackerTestConsts.WorkspaceName });
        result.WorkspaceName.ShouldBe("acme");
    }

    // ---- Workspace subscriptions ----

    [Fact]
    public async Task Workspace_Subscriptions_Should_List_Every_Workspace_With_Its_Plan_Status_And_Seats()
    {
        var beta = await CreateWorkspaceAsync("beta", "Business", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(9));
        var alpha = await CreateWorkspaceAsync("alpha", "Starter", Subscription.ActiveStatus);
        await AddSeatsAsync(alpha, 2);

        var all = (await _platform.GetWorkspaceSubscriptionsAsync()).Items;
        all.Single(i => i.WorkspaceId == TrackerTestConsts.WorkspaceId).SubscriptionId.ShouldBeNull();
        var items = all.Where(i => i.WorkspaceId != TrackerTestConsts.WorkspaceId).ToList();

        items.Select(i => i.WorkspaceName).ShouldBe(new[] { "alpha", "beta" });
        var a = items[0];
        a.WorkspaceId.ShouldBe(alpha);
        a.PlanName.ShouldBe("Starter");
        a.Status.ShouldBe(Subscription.ActiveStatus);
        a.SeatsUsed.ShouldBe(2);
        a.MaxSeats.ShouldBe(25);
        var b = items[1];
        b.WorkspaceId.ShouldBe(beta);
        b.Status.ShouldBe(Subscription.TrialingStatus);
        b.TrialEndsAt.ShouldNotBeNull();
        b.SeatsUsed.ShouldBe(0);
    }

    // ---- Plan lifecycle (host) ----

    [Fact]
    public async Task Changing_A_Trial_To_The_Free_Plan_Should_Activate_It()
    {
        var tenantId = await CreateWorkspaceAsync("trialco", "Starter", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(3));
        var free = await PlanAsync(Plan.FreePlanName);

        var subscription = await _platform.ChangeWorkspacePlanAsync(tenantId, free.Id);

        subscription.PlanId.ShouldBe(free.Id);
        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        subscription.TrialEndsAt.ShouldBeNull();
    }

    [Fact]
    public async Task Changing_To_Another_Paid_Plan_Should_Keep_The_Status()
    {
        var tenantId = await CreateWorkspaceAsync("trialco", "Starter", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(3));
        var business = await PlanAsync("Business");

        var subscription = await _platform.ChangeWorkspacePlanAsync(tenantId, business.Id);

        subscription.PlanId.ShouldBe(business.Id);
        subscription.Status.ShouldBe(Subscription.TrialingStatus);
        (await SubscriptionOfAsync(tenantId)).PlanId.ShouldBe(business.Id);
    }

    [Fact]
    public async Task Changing_The_Plan_Should_Refuse_Unknown_Plans_And_Workspaces()
    {
        var tenantId = await CreateWorkspaceAsync("trialco", "Starter", Subscription.TrialingStatus, DateTime.UtcNow.AddDays(3));
        var business = await PlanAsync("Business");

        (await Should.ThrowAsync<BusinessException>(() => _platform.ChangeWorkspacePlanAsync(tenantId, Guid.NewGuid())))
            .Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
        await Should.ThrowAsync<EntityNotFoundException>(() => _platform.ChangeWorkspacePlanAsync(Guid.NewGuid(), business.Id));
    }

    [Fact]
    public async Task Changing_To_A_Plan_With_Fewer_Seats_Than_Occupied_Should_Be_Refused()
    {
        var tenantId = await CreateWorkspaceAsync("busyco", "Starter", Subscription.ActiveStatus);
        await AddSeatsAsync(tenantId, 4);
        var free = await PlanAsync(Plan.FreePlanName); // 3 seats

        var exception = await Should.ThrowAsync<BusinessException>(() => _platform.ChangeWorkspacePlanAsync(tenantId, free.Id));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        (await SubscriptionOfAsync(tenantId)).PlanId.ShouldBe((await PlanAsync("Starter")).Id);
    }

    [Fact]
    public async Task Changing_The_Plan_Of_A_Workspace_Without_A_Subscription_Should_Start_One()
    {
        var tenantId = await CreateWorkspaceAsync("bareco", "Starter", Subscription.ActiveStatus);
        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _subscriptions.DeleteAsync(s => s.WorkspaceId == tenantId));
        }
        var business = await PlanAsync("Business");

        var subscription = await _platform.ChangeWorkspacePlanAsync(tenantId, business.Id);

        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        (await SubscriptionOfAsync(tenantId)).PlanId.ShouldBe(business.Id);
    }

    [Fact]
    public async Task Activating_An_Ended_Trial_Should_Make_The_Workspace_Usable_And_Billable()
    {
        var tenantId = await CreateWorkspaceAsync("endedco", "Starter", Subscription.PastDueStatus, DateTime.UtcNow.AddDays(-1));

        var subscription = await _platform.ActivateSubscriptionAsync(tenantId);

        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        using (_currentWorkspace.Change(tenantId))
        {
            (await WithUnitOfWorkAsync(() => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeTrue();
        }

        (await GetRequiredService<InvoiceGenerationWorker>().GenerateInvoicesAsync(ServiceProvider)).Generated.ShouldBe(1);
    }

    [Fact]
    public async Task Suspending_Should_Stop_The_Workspace()
    {
        var tenantId = await CreateWorkspaceAsync("badco", "Starter", Subscription.ActiveStatus);

        var subscription = await _platform.SuspendSubscriptionAsync(tenantId);

        subscription.Status.ShouldBe(Subscription.SuspendedStatus);
        using (_currentWorkspace.Change(tenantId))
        {
            (await WithUnitOfWorkAsync(() => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeFalse();
        }
    }

    [Fact]
    public async Task Activating_Or_Suspending_An_Unknown_Workspace_Should_Fail()
    {
        await Should.ThrowAsync<EntityNotFoundException>(() => _platform.ActivateSubscriptionAsync(Guid.NewGuid()));
        await Should.ThrowAsync<EntityNotFoundException>(() => _platform.SuspendSubscriptionAsync(Guid.NewGuid()));
    }

    [Fact]
    public async Task Marking_The_Overdue_Invoice_Paid_Should_Reactivate_The_Workspace()
    {
        var tenantId = await CreateWorkspaceAsync("lateco", "Starter", Subscription.PastDueStatus);
        var invoice = await InvoiceAsync(tenantId, 18m, DateTime.UtcNow.AddDays(-10));

        var paid = await _platform.MarkInvoicePaidAsync(invoice.Id);

        paid.Id.ShouldBe(invoice.Id);
        paid.Status.ShouldBe(Invoice.PaidStatus);
        paid.WorkspaceName.ShouldBe("lateco");
        (await SubscriptionOfAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);

        // Idempotent.
        (await _platform.MarkInvoicePaidAsync(invoice.Id)).Status.ShouldBe(Invoice.PaidStatus);
        (await _platform.GetOverviewAsync()).PendingInvoices.ShouldBe(0);
    }

    [Fact]
    public async Task Marking_An_Unknown_Invoice_Paid_Should_Fail()
    {
        await Should.ThrowAsync<EntityNotFoundException>(() => _platform.MarkInvoicePaidAsync(Guid.NewGuid()));
    }

    // ---- Lifecycle and access ----

    [Fact]
    public async Task Workspace_Sign_Up_Keeps_Its_Own_Plan()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            WorkspaceName = "starterco",
            Email = "owner@starterco.test",
            Password = "1q2w3E*starter",
            PlanName = "Starter"
        });

        using (_currentWorkspace.Change(result.WorkspaceId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var subscription = (await _subscriptions.GetListAsync(s => s.WorkspaceId == result.WorkspaceId)).ShouldHaveSingleItem();
                subscription.Status.ShouldBe("trialing");
                (await _plans.GetAsync(subscription.PlanId)).Name.ShouldBe("Starter");
            });
        }
    }

    [Fact]
    public async Task Workspace_Role_Permissions_Should_Be_Defined()
    {
        var definitions = GetRequiredService<Volo.Abp.Authorization.Permissions.IPermissionDefinitionManager>();

        (await definitions.GetOrNullAsync(Dosi.Tracker.Permissions.TrackerPermissions.Platform.Default)).ShouldNotBeNull();
        foreach (var name in WorkspaceRoles.OwnerPermissions.Concat(WorkspaceRoles.ManagerPermissions))
        {
            (await definitions.GetOrNullAsync(name)).ShouldNotBeNull(name);
        }

        WorkspaceRoles.OwnerPermissions.ShouldNotContain(Dosi.Tracker.Permissions.TrackerPermissions.Platform.Default);
        WorkspaceRoles.ManagerPermissions.ShouldNotContain(Dosi.Tracker.Permissions.TrackerPermissions.Billing.Default);
        WorkspaceRoles.PermissionsOf(WorkspaceRoles.Member).ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Platform_Console_Should_Work_Across_Every_Workspace_Whatever_The_Current_One()
    {
        var alpha = await CreateWorkspaceAsync("alpha", "Starter", Subscription.ActiveStatus);
        await InvoiceAsync(alpha, 7m, DateTime.UtcNow.AddDays(3));

        using (_currentWorkspace.Change(TrackerTestConsts.WorkspaceId))
        {
            (await _platform.GetInvoicesAsync(new PagedAndSortedResultRequestDto())).Items
                .ShouldContain(i => i.WorkspaceId == alpha);
        }

        using (_currentWorkspace.Change(null))
        {
            (await _platform.GetWorkspaceSubscriptionsAsync()).Items.ShouldContain(i => i.WorkspaceId == alpha);
        }
    }
}
