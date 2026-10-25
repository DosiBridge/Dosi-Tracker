using System;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Emailing;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Security.Claims;
using Volo.Abp.Timing;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Billing;

/// <summary>
/// The deployment's billing switch (Billing:Enabled). Off — the self-hosted default — there is no paywall: every
/// workspace is usable, plans limit no seats, sign-up starts nothing on a trial and nothing is swept. On, owners can move
/// to a free plan their team fits and cancel a paid one; archived workspaces are not invoiced; trial notices are emailed.
/// </summary>
// Runs a background sweep, whose named lock is process-wide (ABP's local lock): never alongside the other sweepers.
[Xunit.Collection(TrackerTestConsts.CollectionDefinitionName)]
public class BillingSwitchTests : TrackerEntityFrameworkCoreTestBase
{
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public BillingSwitchTests()
    {
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private void TurnBillingOff() => GetRequiredService<IOptions<BillingOptions>>().Value.Enabled = false;

    private Task<Plan> PlanAsync(string name) => WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == name));

    private async Task<T> InWorkspaceAsync<T>(Guid workspaceId, Func<Task<T>> action)
    {
        using (_currentWorkspace.Change(workspaceId))
        {
            return await WithUnitOfWorkAsync(action);
        }
    }

    /// <summary>A workspace owned by a new account, subscribed to <paramref name="planName"/> in <paramref name="status"/>,
    /// with <paramref name="seats"/> occupied seats (the owner's included when it is at least 1).</summary>
    private async Task<(Guid WorkspaceId, Guid OwnerId)> CreateWorkspaceAsync(
        string name, string planName, string status, int seats = 1, DateTime? trialEndsAt = null)
    {
        var plan = await PlanAsync(planName);
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync(name);
        var users = new Guid[Math.Max(1, seats)];
        for (var i = 0; i < users.Length; i++)
        {
            var userName = $"{name}-{i}@billing.test".Replace(' ', '-').ToLowerInvariant();
            users[i] = await WithUnitOfWorkAsync(async () =>
            {
                var user = new IdentityUser(_guidGenerator.Create(), userName, userName);
                (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
                return user.Id;
            });
            await ServiceProvider.AddWorkspaceMemberAsync(users[i], i == 0 ? WorkspaceRoles.Owner : WorkspaceRoles.Member, workspaceId);
        }

        await InWorkspaceAsync(workspaceId, async () =>
        {
            await _subscriptions.InsertAsync(new Subscription(_guidGenerator.Create(), workspaceId, plan.Id)
            {
                Status = status,
                TrialEndsAt = trialEndsAt
            }, autoSave: true);

            if (seats > 0)
            {
                var projectId = _guidGenerator.Create();
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, workspaceId, "Seats"), autoSave: true);
                foreach (var userId in users)
                {
                    await GetRequiredService<IRepository<ProjectMember, Guid>>()
                        .InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectId, userId), autoSave: true);
                }
            }

            return true;
        });

        return (workspaceId, users[0]);
    }

    private Task<Subscription> SubscriptionOfAsync(Guid workspaceId) =>
        InWorkspaceAsync(workspaceId, () => _subscriptions.FirstAsync(s => s.WorkspaceId == workspaceId));

    private async Task<T> AsOwnerAsync<T>(Guid workspaceId, Guid ownerId, Func<IBillingAppService, Task<T>> action)
    {
        var claims = new[] { new Claim(AbpClaimTypes.UserId, ownerId.ToString()) };
        using (GetRequiredService<ICurrentPrincipalAccessor>().Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "Test"))))
        using (_currentWorkspace.Change(workspaceId, WorkspaceRoles.Owner))
        {
            return await action(GetRequiredService<IBillingAppService>());
        }
    }

    [Fact]
    public async Task With_Billing_Off_Every_Workspace_Is_Usable_And_No_Plan_Limits_Seats()
    {
        var (canceled, _) = await CreateWorkspaceAsync("Off Canceled", "Starter", Subscription.CanceledStatus);
        var (lapsedTrial, _) = await CreateWorkspaceAsync("Off Trial", "Business", Subscription.TrialingStatus,
            trialEndsAt: _clock.Now.AddDays(-30));
        var (crowded, _) = await CreateWorkspaceAsync("Off Crowded", Plan.FreePlanName, Subscription.ActiveStatus, seats: 4);
        TurnBillingOff();

        (await InWorkspaceAsync(canceled, () => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeTrue();
        (await InWorkspaceAsync(lapsedTrial, () => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeTrue();
        await InWorkspaceAsync(crowded, async () =>
        {
            var seats = GetRequiredService<SeatCounter>();
            await seats.CheckSeatAvailableAsync(null); // a fifth person on a 3-seat plan
            await seats.CheckFitsPlanAsync(await PlanAsync(Plan.FreePlanName));
            return true;
        });

        var starter = await PlanAsync("Starter");
        var created = GetRequiredService<SubscriptionManager>().CreateForNewWorkspace(Guid.NewGuid(), starter);
        created.Status.ShouldBe(Subscription.ActiveStatus);
        created.TrialEndsAt.ShouldBeNull();

        (await GetRequiredService<TrialExpiryWorker>().CheckSubscriptionsAsync(ServiceProvider)).BillingDisabled.ShouldBeTrue();
        (await GetRequiredService<InvoiceGenerationWorker>().GenerateInvoicesAsync(ServiceProvider)).BillingDisabled.ShouldBeTrue();
        (await SubscriptionOfAsync(lapsedTrial)).Status.ShouldBe(Subscription.TrialingStatus); // nothing swept

        using (_currentWorkspace.Change(null))
        {
            (await GetRequiredService<IWorkspaceAppService>().GetBillingModeAsync()).Enabled.ShouldBeFalse();
        }
    }

    [Fact]
    public async Task With_Billing_Off_Owners_Have_No_Plan_To_Change_Or_Cancel_And_Sign_Up_Ignores_The_Plan()
    {
        var (workspaceId, ownerId) = await CreateWorkspaceAsync("Off Owner", "Starter", Subscription.ActiveStatus);
        var free = await PlanAsync(Plan.FreePlanName);
        TurnBillingOff();

        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = free.Id }))))
            .Code.ShouldBe(TrackerDomainErrorCodes.BillingDisabled);
        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.CancelAsync())))
            .Code.ShouldBe(TrackerDomainErrorCodes.BillingDisabled);

        WorkspaceRegistrationResultDto registered;
        using (_currentWorkspace.Change(null))
        {
            registered = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
            {
                WorkspaceName = "Self Hosted Co",
                Email = "admin@self-hosted.test",
                Password = "1q2w3E*selfhost",
                PlanName = "No Such Plan"
            });
        }

        var subscription = await SubscriptionOfAsync(registered.WorkspaceId);
        subscription.PlanId.ShouldBe(free.Id);
        subscription.Status.ShouldBe(Subscription.ActiveStatus);
    }

    [Fact]
    public async Task An_Owner_Moves_A_Lapsed_Workspace_To_The_Free_Plan_When_The_Team_Fits()
    {
        var (workspaceId, ownerId) = await CreateWorkspaceAsync("Downgrade Co", "Starter", Subscription.PastDueStatus, seats: 2);
        var free = await PlanAsync(Plan.FreePlanName);

        var changed = await AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = free.Id }));

        changed.PlanId.ShouldBe(free.Id);
        changed.Status.ShouldBe(Subscription.ActiveStatus);
        (await InWorkspaceAsync(workspaceId, () => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeTrue();
    }

    [Fact]
    public async Task An_Owner_Cannot_Move_To_A_Free_Plan_Too_Small_For_The_Team_Or_To_A_Paid_Plan()
    {
        var (workspaceId, ownerId) = await CreateWorkspaceAsync("Big Team Co", "Starter", Subscription.ActiveStatus, seats: 4);
        var free = await PlanAsync(Plan.FreePlanName);
        var business = await PlanAsync("Business");

        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = free.Id }))))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = business.Id }))))
            .Code.ShouldBe(TrackerDomainErrorCodes.PlanChangeNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = Guid.NewGuid() }))))
            .Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);

        (await SubscriptionOfAsync(workspaceId)).PlanId.ShouldBe((await PlanAsync("Starter")).Id);
    }

    [Fact]
    public async Task Cancelling_Pauses_Tracking_Until_The_Owner_Resumes_On_The_Free_Plan()
    {
        var (workspaceId, ownerId) = await CreateWorkspaceAsync("Cancel Co", "Starter", Subscription.ActiveStatus);

        (await AsOwnerAsync(workspaceId, ownerId, b => b.CancelAsync())).Status.ShouldBe(Subscription.CanceledStatus);
        (await AsOwnerAsync(workspaceId, ownerId, b => b.CancelAsync())).Status.ShouldBe(Subscription.CanceledStatus);
        (await InWorkspaceAsync(workspaceId, () => GetRequiredService<ISubscriptionAccessChecker>().IsUsableAsync())).ShouldBeFalse();
        (await GetRequiredService<InvoiceGenerationWorker>().GenerateInvoicesAsync(ServiceProvider)).Generated.ShouldBe(0);

        var free = await PlanAsync(Plan.FreePlanName);
        (await AsOwnerAsync(workspaceId, ownerId, b => b.ChangePlanAsync(new ChangePlanDto { PlanId = free.Id }))).Status
            .ShouldBe(Subscription.ActiveStatus);
        (await Should.ThrowAsync<BusinessException>(() => AsOwnerAsync(workspaceId, ownerId, b => b.CancelAsync())))
            .Code.ShouldBe(TrackerDomainErrorCodes.PlanChangeNotAllowed);
    }

    [Fact]
    public async Task An_Archived_Workspace_Is_Not_Invoiced()
    {
        var (archived, ownerId) = await CreateWorkspaceAsync("Archived Billing Co", "Starter", Subscription.ActiveStatus);
        var (active, _) = await CreateWorkspaceAsync("Active Billing Co", "Starter", Subscription.ActiveStatus);
        await WithUnitOfWorkAsync(() => GetRequiredService<WorkspaceManager>().ArchiveAsync(archived, ownerId));

        var result = await GetRequiredService<InvoiceGenerationWorker>().GenerateInvoicesAsync(ServiceProvider);

        result.Generated.ShouldBe(1);
        var invoices = await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await GetRequiredService<IRepository<Invoice, Guid>>().GetListAsync();
            }
        });
        invoices.ShouldContain(i => i.WorkspaceId == active);
        invoices.ShouldNotContain(i => i.WorkspaceId == archived);
    }

    [Fact]
    public async Task The_Owners_Are_Emailed_Before_And_When_A_Trial_Ends()
    {
        var (ending, _) = await CreateWorkspaceAsync("Trial Ending Co", "Starter", Subscription.TrialingStatus,
            trialEndsAt: _clock.Now.AddDays(1));
        var (ended, _) = await CreateWorkspaceAsync("Trial Ended Co", "Starter", Subscription.TrialingStatus,
            trialEndsAt: _clock.Now.AddDays(-1));
        var emails = GetRequiredService<CapturingEmailSender>();

        var result = await GetRequiredService<TrialExpiryWorker>().CheckSubscriptionsAsync(ServiceProvider);

        result.TrialRemindersSent.ShouldBe(1);
        result.TrialsEnded.ShouldBe(1);
        emails.To("trial-ending-co-0@billing.test").ShouldHaveSingleItem().Subject
            .ShouldStartWith("Your Starter trial of Trial Ending Co ends on ");
        emails.To("trial-ended-co-0@billing.test").ShouldHaveSingleItem().Subject
            .ShouldBe("Your Starter trial of Trial Ended Co has ended");
        emails.Emails.Count.ShouldBe(2); // owners only, once each
        (await SubscriptionOfAsync(ended)).Status.ShouldBe(Subscription.PastDueStatus);
        (await SubscriptionOfAsync(ending)).IsTrialReminderSent().ShouldBeTrue();
    }
}
