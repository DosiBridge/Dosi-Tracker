using System;
using System.Collections.Generic;
using System.Globalization;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class SubscriptionManagerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly SubscriptionManager _manager;
    private readonly ISubscriptionAccessChecker _accessChecker;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Invoice, Guid> _invoices;
    private readonly IRepository<Notification, Guid> _notifications;
    private readonly IRepository<Project, Guid> _projects;
    private readonly IRepository<ProjectMember, Guid> _members;
    private readonly IdentityUserManager _userManager;
    private readonly IdentityRoleManager _roleManager;
    private readonly ICurrentTenant _currentTenant;
    private readonly IGuidGenerator _guidGenerator;

    protected SubscriptionManagerTests()
    {
        _manager = GetRequiredService<SubscriptionManager>();
        _accessChecker = GetRequiredService<ISubscriptionAccessChecker>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _invoices = GetRequiredService<IRepository<Invoice, Guid>>();
        _notifications = GetRequiredService<IRepository<Notification, Guid>>();
        _projects = GetRequiredService<IRepository<Project, Guid>>();
        _members = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _roleManager = GetRequiredService<IdentityRoleManager>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private async Task<T> InWorkspaceAsync<T>(Guid? tenantId, Func<Task<T>> action)
    {
        using (_currentTenant.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(action);
        }
    }

    private async Task InWorkspaceAsync(Guid? tenantId, Func<Task> action)
    {
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(action);
        }
    }

    private Task<Plan> PlanAsync(string name)
    {
        return InWorkspaceAsync(null, () => _plans.FirstAsync(p => p.Name == name));
    }

    private Task<Subscription> SubscribeAsync(Guid tenantId, Plan plan, string status, DateTime? trialEndsAt = null)
    {
        return InWorkspaceAsync(tenantId, () => _subscriptions.InsertAsync(
            new Subscription(_guidGenerator.Create(), tenantId, plan.Id) { Status = status, TrialEndsAt = trialEndsAt }));
    }

    private Task<Subscription> GetSubscriptionAsync(Guid tenantId)
    {
        return InWorkspaceAsync(tenantId, () => _subscriptions.FirstAsync(s => s.TenantId == tenantId));
    }

    private Task<Invoice> InvoiceAsync(Guid tenantId, DateTime dueDate, decimal amount = 12m)
    {
        return InWorkspaceAsync(tenantId, () => _invoices.InsertAsync(new Invoice(_guidGenerator.Create(), tenantId, amount, dueDate)));
    }

    private Task<SubscriptionReviewOutcome> ReviewAsync(Guid tenantId)
    {
        return InWorkspaceAsync(tenantId, async () =>
            await _manager.ReviewAsync(await _subscriptions.FirstAsync(s => s.TenantId == tenantId)));
    }

    /// <summary>A workspace owner: an account in the workspace's "admin" role.</summary>
    private Task<Guid> CreateOwnerAsync(Guid tenantId, string userName, bool active = true)
    {
        return InWorkspaceAsync(tenantId, async () =>
        {
            if (await _roleManager.FindByNameAsync(TrackerRoles.Admin) == null)
            {
                (await _roleManager.CreateAsync(new IdentityRole(_guidGenerator.Create(), TrackerRoles.Admin, tenantId))).Succeeded.ShouldBeTrue();
            }

            var user = new IdentityUser(_guidGenerator.Create(), userName, $"{userName}@owner.test", tenantId);
            (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
            (await _userManager.AddToRoleAsync(user, TrackerRoles.Admin)).Succeeded.ShouldBeTrue();
            if (!active)
            {
                user.SetIsActive(false);
                (await _userManager.UpdateAsync(user)).Succeeded.ShouldBeTrue();
            }

            return user.Id;
        });
    }

    private Task<List<Notification>> NotificationsOfAsync(Guid tenantId, Guid userId)
    {
        return InWorkspaceAsync(tenantId, () => _notifications.GetListAsync(n => n.UserId == userId));
    }

    // ---- How a workspace starts ----

    [Fact]
    public async Task A_Paid_Plan_With_A_Trial_Should_Start_Trialing()
    {
        var starter = await PlanAsync("Starter");

        var subscription = _manager.CreateForNewWorkspace(Guid.NewGuid(), starter);

        subscription.Status.ShouldBe(Subscription.TrialingStatus);
        subscription.TrialEndsAt.ShouldNotBeNull();
        subscription.TrialEndsAt.Value.ShouldBeInRange(DateTime.UtcNow.AddDays(14).AddMinutes(-1), DateTime.UtcNow.AddDays(14).AddMinutes(1));
    }

    [Theory]
    [InlineData(0, 0)]   // the Free plan
    [InlineData(0, 30)]  // a free plan with trial days: nothing to trial
    [InlineData(9, 0)]   // a paid plan without a trial (assigned by the host)
    public void Other_Plans_Should_Start_Active(int pricePerUser, int trialDays)
    {
        var plan = new Plan(Guid.NewGuid(), "Custom", pricePerUser, 10, trialDays);

        var subscription = _manager.CreateForNewWorkspace(Guid.NewGuid(), plan);

        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        subscription.TrialEndsAt.ShouldBeNull();
        subscription.PlanId.ShouldBe(plan.Id);
    }

    // ---- Host actions ----

    [Fact]
    public async Task Changing_A_Trial_To_The_Free_Plan_Should_Activate_It()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.TrialingStatus, DateTime.UtcNow.AddDays(5));
        var free = await PlanAsync(Plan.FreePlanName);

        var changed = await InWorkspaceAsync(tenantId, () => _manager.ChangePlanAsync(free));

        changed.PlanId.ShouldBe(free.Id);
        changed.Status.ShouldBe(Subscription.ActiveStatus);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);
    }

    [Fact]
    public async Task Changing_A_Trial_To_Another_Paid_Plan_Should_Keep_The_Trial()
    {
        var tenantId = Guid.NewGuid();
        var trialEndsAt = DateTime.UtcNow.AddDays(5);
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.TrialingStatus, trialEndsAt);
        var business = await PlanAsync("Business");

        await InWorkspaceAsync(tenantId, () => _manager.ChangePlanAsync(business));

        var subscription = await GetSubscriptionAsync(tenantId);
        subscription.PlanId.ShouldBe(business.Id);
        subscription.Status.ShouldBe(Subscription.TrialingStatus);
        subscription.TrialEndsAt.ShouldNotBeNull();
    }

    [Fact]
    public async Task Changing_The_Plan_Of_A_Workspace_Without_A_Subscription_Should_Start_An_Active_One()
    {
        var tenantId = Guid.NewGuid();
        var business = await PlanAsync("Business");

        var created = await InWorkspaceAsync(tenantId, () => _manager.ChangePlanAsync(business));

        created.TenantId.ShouldBe(tenantId);
        created.Status.ShouldBe(Subscription.ActiveStatus);
        (await GetSubscriptionAsync(tenantId)).PlanId.ShouldBe(business.Id);
    }

    [Fact]
    public async Task A_Plan_Change_Should_Be_Refused_When_The_Workspace_Has_Too_Many_Seats()
    {
        var tenantId = Guid.NewGuid();
        var starter = await PlanAsync("Starter");
        await SubscribeAsync(tenantId, starter, Subscription.ActiveStatus);
        var projectId = await InWorkspaceAsync(tenantId, async () =>
            (await _projects.InsertAsync(new Project(_guidGenerator.Create(), tenantId, "Busy"))).Id);
        for (var i = 0; i < 4; i++)
        {
            // Four people of the workspace (seats are held by accounts) tracking on the project.
            await InWorkspaceAsync(tenantId, async () =>
            {
                var user = new IdentityUser(_guidGenerator.Create(), $"busy{i}", $"busy{i}@busy.test", tenantId);
                (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
                await _members.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, projectId, user.Id));
            });
        }

        var free = await PlanAsync(Plan.FreePlanName); // 3 seats
        var exception = await Should.ThrowAsync<BusinessException>(
            () => InWorkspaceAsync(tenantId, () => _manager.ChangePlanAsync(free)));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        (await GetSubscriptionAsync(tenantId)).PlanId.ShouldBe(starter.Id);
    }

    [Fact]
    public async Task Activate_And_Suspend_Should_Change_What_The_Workspace_May_Do()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.PastDueStatus, DateTime.UtcNow.AddDays(-2));
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeFalse();

        var activated = await InWorkspaceAsync(tenantId, () => _manager.ActivateAsync());
        activated.Status.ShouldBe(Subscription.ActiveStatus);
        activated.TrialEndsAt.ShouldBeNull();
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeTrue();

        var suspended = await InWorkspaceAsync(tenantId, () => _manager.SuspendAsync());
        suspended.Status.ShouldBe(Subscription.SuspendedStatus);
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeFalse();
    }

    [Fact]
    public async Task Activating_A_Workspace_Without_A_Subscription_Should_Fail()
    {
        await Should.ThrowAsync<EntityNotFoundException>(
            () => InWorkspaceAsync(Guid.NewGuid(), () => _manager.ActivateAsync()));
    }

    // ---- Payments ----

    [Fact]
    public async Task Paying_The_Last_Overdue_Invoice_Should_Reactivate_A_Past_Due_Workspace()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.PastDueStatus);
        var overdue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-3));
        var notYetDue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(10));

        var reactivated = await InWorkspaceAsync(tenantId, async () =>
            await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(overdue.Id)));

        reactivated.ShouldBeTrue();
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);
        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(overdue.Id))).Status.ShouldBe(Invoice.PaidStatus);
        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(notYetDue.Id))).Status.ShouldBe(Invoice.PendingStatus);
    }

    [Fact]
    public async Task Paying_One_Of_Several_Overdue_Invoices_Should_Keep_The_Workspace_Past_Due()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.PastDueStatus);
        var first = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-40));
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-3));

        var reactivated = await InWorkspaceAsync(tenantId, async () =>
            await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(first.Id)));

        reactivated.ShouldBeFalse();
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.PastDueStatus);
    }

    [Fact]
    public async Task Paying_An_Invoice_That_Is_Not_Overdue_Should_Not_Reactivate()
    {
        // Past due for another reason (its trial ended): an advance payment alone does not activate it.
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.PastDueStatus, DateTime.UtcNow.AddDays(-1));
        var notYetDue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(5));

        var reactivated = await InWorkspaceAsync(tenantId, async () =>
            await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(notYetDue.Id)));

        reactivated.ShouldBeFalse();
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.PastDueStatus);
    }

    [Fact]
    public async Task Paying_A_Paid_Invoice_Again_Should_Change_Nothing()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.ActiveStatus);
        var invoice = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-3));
        await InWorkspaceAsync(tenantId, async () => await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(invoice.Id)));

        var again = await InWorkspaceAsync(tenantId, async () =>
            await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(invoice.Id)));

        again.ShouldBeFalse();
        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(invoice.Id))).Status.ShouldBe(Invoice.PaidStatus);
    }

    // ---- The hourly review ----

    [Fact]
    public async Task Owners_Should_Be_Reminded_Once_Before_The_Trial_Ends()
    {
        var tenantId = Guid.NewGuid();
        var owner = await CreateOwnerAsync(tenantId, "owner");
        var formerOwner = await CreateOwnerAsync(tenantId, "former", active: false);
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.TrialingStatus, DateTime.UtcNow.AddDays(2));

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.TrialReminderSent);
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);

        var subscription = await GetSubscriptionAsync(tenantId);
        subscription.Status.ShouldBe(Subscription.TrialingStatus);
        subscription.IsTrialReminderSent().ShouldBeTrue();

        var reminders = await NotificationsOfAsync(tenantId, owner);
        reminders.ShouldHaveSingleItem().Message.ShouldContain("Starter trial ends on");
        (await NotificationsOfAsync(tenantId, formerOwner)).ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Trial_Far_From_Its_End_Should_Be_Left_Alone()
    {
        var tenantId = Guid.NewGuid();
        var owner = await CreateOwnerAsync(tenantId, "owner");
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.TrialingStatus, DateTime.UtcNow.AddDays(10));

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);
        (await NotificationsOfAsync(tenantId, owner)).ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Ended_Trial_Should_Become_Past_Due_And_Tell_The_Owners()
    {
        var tenantId = Guid.NewGuid();
        var owner = await CreateOwnerAsync(tenantId, "owner");
        await SubscribeAsync(tenantId, await PlanAsync("Business"), Subscription.TrialingStatus, DateTime.UtcNow.AddMinutes(-5));

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.TrialEnded);

        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.PastDueStatus);
        (await NotificationsOfAsync(tenantId, owner)).ShouldHaveSingleItem().Message.ShouldContain("Business trial has ended");
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeFalse();
    }

    [Fact]
    public async Task An_Overdue_Invoice_Should_Make_An_Active_Workspace_Past_Due_Once()
    {
        var tenantId = Guid.NewGuid();
        var owner = await CreateOwnerAsync(tenantId, "owner");
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.ActiveStatus);
        var overdue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-1), amount: 48m);

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.MarkedPastDue);

        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.PastDueStatus);
        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(overdue.Id))).IsOverdueNoticeSent().ShouldBeTrue();
        (await NotificationsOfAsync(tenantId, owner)).ShouldHaveSingleItem().Message.ShouldContain("48.00");
        // Within the grace period the workspace keeps working.
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeTrue();

        // The host activates it again without the invoice being paid: the same invoice does not suspend it anew...
        await InWorkspaceAsync(tenantId, () => _manager.ActivateAsync());
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);

        // ...but the next invoice that goes unpaid does.
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddHours(-1));
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.MarkedPastDue);
        (await NotificationsOfAsync(tenantId, owner)).Count.ShouldBe(2);
    }

    [Fact]
    public async Task An_Invoice_The_Host_Forgave_Should_Neither_Shorten_The_Next_Grace_Nor_Block_Reactivation()
    {
        var tenantId = Guid.NewGuid();
        var owner = await CreateOwnerAsync(tenantId, "owner");
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.ActiveStatus);
        var forgiven = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-45), amount: 24m);
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.MarkedPastDue);
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeFalse(); // its grace is long over

        // The host settles out of band and reactivates the workspace without marking the invoice paid.
        await InWorkspaceAsync(tenantId, () => _manager.ActivateAsync());
        var excused = await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(forgiven.Id));
        excused.IsOverdueExcused().ShouldBeTrue();
        excused.Status.ShouldBe(Invoice.PendingStatus); // still owed
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeTrue();

        // The next invoice goes unpaid: past due again, with that invoice's full grace period, and the owners are told
        // a date in the future.
        var next = await InvoiceAsync(tenantId, DateTime.UtcNow.AddHours(-1), amount: 36m);
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.MarkedPastDue);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.PastDueStatus);
        (await InWorkspaceAsync(tenantId, () => _accessChecker.IsUsableAsync())).ShouldBeTrue();

        var graceEnd = next.DueDate.AddDays(Subscription.PastDueGraceDays);
        (await InWorkspaceAsync(tenantId, () => GetRequiredService<SubscriptionAccessChecker>().GetPastDueGraceEndAsync()))
            .ShouldBe(graceEnd);
        (await NotificationsOfAsync(tenantId, owner)).ShouldContain(n =>
            n.Message.Contains("36.00") &&
            n.Message.Contains(graceEnd.ToString("MMMM d, yyyy", CultureInfo.InvariantCulture)));

        // Paying that invoice reactivates the workspace, although the forgiven one is still unpaid.
        (await InWorkspaceAsync(tenantId, async () => await _manager.MarkInvoicePaidAsync(await _invoices.GetAsync(next.Id))))
            .ShouldBeTrue();
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);
    }

    [Fact]
    public async Task Activating_Should_Excuse_An_Overdue_Invoice_The_Sweep_Has_Not_Seen_Yet()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.PastDueStatus, DateTime.UtcNow.AddDays(-20));
        var overdue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-1));
        var notYetDue = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(5));

        await InWorkspaceAsync(tenantId, () => _manager.ActivateAsync());

        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(overdue.Id))).IsOverdueExcused().ShouldBeTrue();
        (await InWorkspaceAsync(tenantId, () => _invoices.GetAsync(notYetDue.Id))).IsOverdueExcused().ShouldBeFalse();

        // The next sweep does not suspend the workspace again for the invoice the host excused.
        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);
    }

    [Fact]
    public async Task Invoices_That_Are_Paid_Or_Not_Yet_Due_Should_Not_Make_A_Workspace_Past_Due()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), Subscription.ActiveStatus);
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(3));
        var paid = await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-20));
        await InWorkspaceAsync(tenantId, async () =>
        {
            var invoice = await _invoices.GetAsync(paid.Id);
            invoice.MarkPaid();
            await _invoices.UpdateAsync(invoice);
        });

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(Subscription.ActiveStatus);
    }

    [Theory]
    [InlineData(Subscription.PastDueStatus)]
    [InlineData(Subscription.SuspendedStatus)]
    [InlineData(Subscription.CanceledStatus)]
    public async Task Other_Statuses_Should_Be_Left_Alone(string status)
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, await PlanAsync("Starter"), status, DateTime.UtcNow.AddDays(-30));
        await InvoiceAsync(tenantId, DateTime.UtcNow.AddDays(-3));

        (await ReviewAsync(tenantId)).ShouldBe(SubscriptionReviewOutcome.None);
        (await GetSubscriptionAsync(tenantId)).Status.ShouldBe(status);
    }
}
