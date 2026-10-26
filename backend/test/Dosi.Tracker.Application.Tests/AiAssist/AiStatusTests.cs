using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// <c>GET /api/app/ai-assist/status</c> per role and per failing check (it never throws for AI reasons), and the usage
/// view, both summed from the ledger.
/// </summary>
public abstract class AiStatusTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IAiAssistAppService _service;

    protected AiStatusTests()
    {
        _service = GetRequiredService<IAiAssistAppService>();
    }

    private Task<AiStatusDto> StatusAsync() => _service.GetStatusAsync();

    private static AiFeatureStatusDto Feature(AiStatusDto status, string featureId) =>
        status.Features.Single(f => f.FeatureId == featureId);

    [Fact]
    public async Task An_Owner_Of_An_Enabled_Workspace_Should_Have_Every_Feature_And_The_Workspace_Allowance()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync(titles: true);
        await InsertUsageAsync(CallerId, inputTokens: 1000, outputTokens: 200);

        var status = await StatusAsync();

        status.Available.ShouldBeTrue();
        status.Reason.ShouldBeNull();
        status.WorkspaceEnabled.ShouldBeTrue();
        status.CanManage.ShouldBeTrue();
        status.ProviderDisplayName.ShouldBe("OpenAI");
        status.DisclosureVersion.ShouldBe("1.0");
        status.AllowWindowTitles.ShouldBeTrue();
        status.AllowNotes.ShouldBeFalse();
        status.Features.Select(f => f.FeatureId).ShouldBe(AiFeatureIds.All.Select(f => f.Id));
        status.Features.ShouldAllBe(f => f.Allowed && f.Reason == null);
        Feature(status, AiFeatureIds.PeriodSummaryId).Scopes.ShouldBe(new[] { "team", "self" });
        Feature(status, AiFeatureIds.MemberDaySummaryId).Scopes.ShouldBe(new[] { "member", "self" });
        Feature(status, AiFeatureIds.DataQualityReviewId).Scopes.ShouldBe(new[] { "team", "self" });
        Feature(status, AiFeatureIds.AskId).Scopes.ShouldBeEmpty();

        var quota = status.Quota.ShouldNotBeNull();
        quota.RequestsUsed.ShouldBe(1);
        quota.TokensUsed.ShouldBe(1200);
        quota.RequestsLimit.ShouldBe(AiOptions.Quotas.Default.MonthlyRequests);
        quota.TokensLimit.ShouldBe(AiOptions.Quotas.Default.MonthlyTokens);
        quota.ResetsAt.ShouldBe(AiUsageRecordQueries.MonthStart(Now).AddMonths(1));
        Provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Manager_Should_See_The_Allowance_But_Not_Manage()
    {
        await SignInAsAsync(WorkspaceRoles.Manager);
        await SetPolicyAsync();

        var status = await StatusAsync();

        status.Available.ShouldBeTrue();
        status.CanManage.ShouldBeFalse();
        status.Quota.ShouldNotBeNull();
        Feature(status, AiFeatureIds.ReportExplanationId).Allowed.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Member_Should_Get_Self_Features_Only_And_No_Allowance()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await AddProjectMembershipAsync(CallerId, ProjectMemberRoles.Worker);
        await SetPolicyAsync();

        var status = await StatusAsync();

        status.Available.ShouldBeTrue();
        status.CanManage.ShouldBeFalse();
        status.Quota.ShouldBeNull();
        var report = Feature(status, AiFeatureIds.ReportExplanationId);
        report.Allowed.ShouldBeFalse();
        report.Reason.ShouldBe(AiFeatureReasons.Role);
        report.Scopes.ShouldBeEmpty();
        Feature(status, AiFeatureIds.PeriodSummaryId).Scopes.ShouldBe(new[] { "self" });
        Feature(status, AiFeatureIds.MemberDaySummaryId).Scopes.ShouldBe(new[] { "self" });
        Feature(status, AiFeatureIds.TaskDraftsId).Allowed.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Client_Only_Member_Should_See_Every_Feature_Disallowed_For_Their_Role()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await AddProjectMembershipAsync(CallerId, ProjectMemberRoles.Client);
        await SetPolicyAsync();

        var status = await StatusAsync();

        status.Available.ShouldBeFalse();
        status.Reason.ShouldBe(AiStatusReasons.RoleNotAllowed);
        status.WorkspaceEnabled.ShouldBeTrue();
        status.Features.ShouldAllBe(f => !f.Allowed && f.Reason == AiFeatureReasons.Role && f.Scopes.Count == 0);
        status.Quota.ShouldBeNull();
    }

    [Fact]
    public async Task Managers_The_Owner_Does_Not_Allow_And_Members_Without_Self_Service_Should_Be_Told_Why()
    {
        await SignInAsAsync(WorkspaceRoles.Manager);
        await SetPolicyAsync(managers: false, selfService: false);
        var member = await CreateMemberAsync("ai.member", WorkspaceRoles.Member);

        var managerStatus = await StatusAsync();
        managerStatus.Reason.ShouldBe(AiStatusReasons.RoleNotAllowed);
        managerStatus.Features.ShouldAllBe(f => !f.Allowed && f.Reason == AiFeatureReasons.Role);

        var memberStatus = await AsUserAsync(member, StatusAsync);
        memberStatus.Reason.ShouldBe(AiStatusReasons.SelfServiceOff);
        memberStatus.Features.ShouldAllBe(f => !f.Allowed && f.Reason == AiFeatureReasons.SelfServiceOff);
    }

    [Fact]
    public async Task A_Workspace_That_Is_Off_Should_Say_So_With_Every_Feature_Disallowed()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);

        var status = await StatusAsync();

        status.Available.ShouldBeFalse();
        status.Reason.ShouldBe(AiStatusReasons.WorkspaceDisabled);
        status.WorkspaceEnabled.ShouldBeFalse();
        status.CanManage.ShouldBeTrue();
        status.ProviderDisplayName.ShouldBe("OpenAI");
        status.Features.ShouldAllBe(f => !f.Allowed && f.Reason == null);
    }

    [Fact]
    public async Task A_Changed_Disclosure_Should_Ask_For_Consent_Again()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        AiOptions.Disclosure.Version = 4;

        var status = await StatusAsync();

        status.Reason.ShouldBe(AiStatusReasons.ReconsentRequired);
        status.WorkspaceEnabled.ShouldBeTrue();
        status.DisclosureVersion.ShouldBe("1.4");
        status.Features.ShouldAllBe(f => !f.Allowed);
    }

    [Fact]
    public async Task The_Checks_Should_Be_Reported_In_Order()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetSubscriptionAsync(Subscription.SuspendedStatus);
        await SetPlatformEnabledAsync(false);

        (await StatusAsync()).Reason.ShouldBe(AiStatusReasons.PlatformDisabled);

        await SetPlatformEnabledAsync(true);
        (await StatusAsync()).Reason.ShouldBe(AiStatusReasons.SubscriptionInactive);

        Provider.State = AiProviderState.MissingModel;
        var notConfigured = await StatusAsync();
        notConfigured.Reason.ShouldBe(AiStatusReasons.NotConfigured);
        notConfigured.ProviderDisplayName.ShouldBeNull();
    }

    [Fact]
    public async Task A_Feature_The_Operator_Switched_Off_Should_Be_Disallowed_On_Its_Own()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        AiOptions.Features["TaskDrafts"] = new AiFeatureOptions { Enabled = false };

        var status = await StatusAsync();

        status.Available.ShouldBeTrue();
        var drafts = Feature(status, AiFeatureIds.TaskDraftsId);
        drafts.Allowed.ShouldBeFalse();
        drafts.Reason.ShouldBe(AiFeatureReasons.Operator);
        status.Features.Where(f => f.FeatureId != AiFeatureIds.TaskDraftsId).ShouldAllBe(f => f.Allowed);
    }

    [Fact]
    public async Task A_Used_Up_Daily_Allowance_Should_Exhaust_AI_For_That_Member_Only()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        var member = await CreateMemberAsync("ai.member", WorkspaceRoles.Member);
        var daily = AiOptions.Quotas.Default.UserDailyRequests!.Value;

        for (var i = 0; i < daily; i++)
        {
            await InsertUsageAsync(member);
        }

        // Rows that never reached the provider do not count.
        await InsertUsageAsync(CallerId, outcome: AiUsageOutcomes.Cached, cacheHit: true);
        await InsertUsageAsync(CallerId, outcome: AiUsageOutcomes.NoData);
        await InsertUsageAsync(CallerId, outcome: AiUsageOutcomes.Clarification);
        // Nor do yesterday's.
        for (var i = 0; i < daily; i++)
        {
            await InsertUsageAsync(CallerId, at: AiUsageRecordQueries.DayStart(Now).AddMinutes(-1 - i));
        }

        var memberStatus = await AsUserAsync(member, StatusAsync);
        memberStatus.Available.ShouldBeFalse();
        memberStatus.Reason.ShouldBe(AiStatusReasons.QuotaExhausted);
        memberStatus.Features.ShouldAllBe(f => !f.Allowed);

        (await StatusAsync()).Available.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Used_Up_Monthly_Allowance_Should_Exhaust_AI_For_The_Workspace()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        AiOptions.Quotas.Default.MonthlyTokens = 5000;
        await InsertUsageAsync(Guid.NewGuid(), inputTokens: 4000, outputTokens: 1000);
        // Another workspace's usage is not this one's.
        await InsertUsageAsync(Guid.NewGuid(), inputTokens: 90000, workspaceId: Guid.NewGuid());

        var status = await StatusAsync();

        status.Reason.ShouldBe(AiStatusReasons.QuotaExhausted);
        status.Quota!.TokensUsed.ShouldBe(5000);
        status.Quota.TokensLimit.ShouldBe(5000);
    }

    [Fact]
    public async Task The_Plan_Should_Set_The_Allowance()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        await SetSubscriptionAsync(Subscription.ActiveStatus, planName: "starter");

        var quota = (await StatusAsync()).Quota.ShouldNotBeNull();

        quota.RequestsLimit.ShouldBe(500);
        quota.TokensLimit.ShouldBe(2_000_000);
    }

    [Fact]
    public async Task Owners_Should_See_The_Workspace_Usage_With_Cost_And_Others_Only_Their_Own()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        var manager = await CreateMemberAsync("ai.manager", WorkspaceRoles.Manager);
        await InsertUsageAsync(CallerId, AiFeatureIds.PeriodSummaryId, inputTokens: 1000, outputTokens: 100, cost: 0.0015m);
        await InsertUsageAsync(manager, AiFeatureIds.AskId, inputTokens: 2000, outputTokens: 300, cost: 0.0025m);
        await InsertUsageAsync(manager, AiFeatureIds.AskId, inputTokens: 500, outputTokens: 50);
        await InsertUsageAsync(manager, AiFeatureIds.AskId, AiUsageOutcomes.Cached, cacheHit: true, inputTokens: 999, outputTokens: 999);
        // Last month's usage is not this month's.
        await InsertUsageAsync(manager, at: AiUsageRecordQueries.MonthStart(Now).AddSeconds(-1), cost: 9m);

        var owner = await _service.GetUsageAsync();

        owner.MonthStart.ShouldBe(AiUsageRecordQueries.MonthStart(Now));
        owner.ResetsAt.ShouldBe(owner.MonthStart.AddMonths(1));
        owner.RequestsUsed.ShouldBe(3);
        owner.TokensUsed.ShouldBe(1000 + 100 + 2000 + 300 + 500 + 50);
        owner.RequestsLimit.ShouldBe(AiOptions.Quotas.Default.MonthlyRequests);
        owner.EstimatedCostUsd.ShouldBe(0.004m);
        owner.ByFeature.Select(f => (f.FeatureId, f.Requests, f.Tokens)).ShouldBe(new[]
        {
            (AiFeatureIds.AskId, 2, 2850L),
            (AiFeatureIds.PeriodSummaryId, 1, 1100L)
        });
        owner.MyRequestsToday.ShouldBe(1);
        owner.MyDailyLimit.ShouldBe(AiOptions.Quotas.Default.UserDailyRequests);

        var mine = await AsUserAsync(manager, () => _service.GetUsageAsync());
        mine.RequestsUsed.ShouldBe(2);
        mine.TokensUsed.ShouldBe(2850);
        mine.RequestsLimit.ShouldBeNull();
        mine.TokensLimit.ShouldBeNull();
        mine.EstimatedCostUsd.ShouldBeNull();
        mine.ByFeature.ShouldBeEmpty();
        mine.MyRequestsToday.ShouldBe(2);
    }

    [Fact]
    public async Task Usage_Without_Priced_Requests_Should_Have_No_Cost()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await InsertUsageAsync(CallerId);

        var usage = await _service.GetUsageAsync();

        usage.RequestsUsed.ShouldBe(1);
        usage.EstimatedCostUsd.ShouldBeNull();
    }
}

/// <summary>Status on a server without an AI provider (the application's default, as on every test host).</summary>
public abstract class AiStatusWithoutProviderTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override bool UseFakeProvider => false;

    [Fact]
    public async Task Status_Should_Say_Not_Configured_And_Allow_Nothing()
    {
        GetRequiredService<IAiChatClientProvider>().ShouldBeOfType<NullAiChatClientProvider>();
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();

        var status = await GetRequiredService<IAiAssistAppService>().GetStatusAsync();

        status.Available.ShouldBeFalse();
        status.Reason.ShouldBe(AiStatusReasons.NotConfigured);
        status.ProviderDisplayName.ShouldBeNull();
        status.WorkspaceEnabled.ShouldBeTrue();
        status.Features.Count.ShouldBe(AiFeatureIds.All.Count);
        status.Features.ShouldAllBe(f => !f.Allowed);
    }
}
