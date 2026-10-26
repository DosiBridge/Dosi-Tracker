using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The refusal matrix of <see cref="AiAccessGuard"/>: every check, its error code and data, and the order they are
/// checked in. No refusal ever reaches the provider.
/// </summary>
public abstract class AiAccessGuardTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly AiAccessGuard _guard;

    protected AiAccessGuardTests()
    {
        _guard = GetRequiredService<AiAccessGuard>();
    }

    private Task<AiCallerContext> CallerAsync() =>
        WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync());

    private async Task<BusinessException> RefusedAsync(string featureId, AiCallerContext? caller = null)
    {
        var context = caller ?? await CallerAsync();
        var refused = await Should.ThrowAsync<BusinessException>(() => WithUnitOfWorkAsync(() => _guard.EnsureAsync(context, featureId)));
        refused.Data["featureId"].ShouldBe(featureId);
        Provider.Client.Calls.Count.ShouldBe(0);
        Provider.RequestedFeatures.ShouldBeEmpty();
        return refused;
    }

    private async Task AllowedAsync(string featureId, AiCallerContext? caller = null)
    {
        var context = caller ?? await CallerAsync();
        await WithUnitOfWorkAsync(() => _guard.EnsureAsync(context, featureId));
        Provider.Client.Calls.Count.ShouldBe(0);
    }

    [Fact]
    public async Task An_Owner_Of_An_Enabled_Workspace_Should_Pass_For_Every_Feature()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();

        foreach (var feature in AiFeatureIds.All)
        {
            await AllowedAsync(feature.Id);
        }
    }

    [Fact]
    public async Task A_Server_Without_A_Provider_Should_Refuse_First()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        Provider.State = AiProviderState.MissingApiKey;
        // Every later check would refuse too.
        AiOptions.Features["PeriodSummary"] = new AiFeatureOptions { Enabled = false };
        await SetPlatformEnabledAsync(false);

        (await RefusedAsync(AiFeatureIds.PeriodSummaryId)).Code.ShouldBe(TrackerDomainErrorCodes.AiNotConfigured);
    }

    [Fact]
    public async Task A_Feature_The_Operator_Switched_Off_Should_Be_Refused_Before_The_Platform_Switch()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        AiOptions.Features["periodsummary"] = new AiFeatureOptions { Enabled = false };
        await SetPlatformEnabledAsync(false);

        var refused = await RefusedAsync(AiFeatureIds.PeriodSummaryId);

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
        refused.Data["reason"].ShouldBe(AiFeatureReasons.Operator);
        // The other features fall through to the platform switch.
        (await RefusedAsync(AiFeatureIds.MemberDaySummaryId)).Code.ShouldBe(TrackerDomainErrorCodes.AiPlatformDisabled);
    }

    [Theory]
    [InlineData(false, true, true)]
    [InlineData(true, false, true)]
    [InlineData(true, true, false)]
    public async Task Ask_AI_Should_Be_Off_Without_The_Assistant_Or_Tool_Support(bool assistant, bool supportsTools, bool askEnabled)
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        AiOptions.Assistant.Enabled = assistant;
        AiOptions.SupportsTools = supportsTools;
        AiOptions.Features["Ask"] = new AiFeatureOptions { Enabled = askEnabled };

        (await RefusedAsync(AiFeatureIds.AskId)).Data["reason"].ShouldBe(AiFeatureReasons.Operator);
        await AllowedAsync(AiFeatureIds.PeriodSummaryId);
    }

    [Fact]
    public async Task The_Platform_Switch_Should_Refuse_Before_The_Subscription()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetSubscriptionAsync(Subscription.CanceledStatus);
        await SetPlatformEnabledAsync(false);

        (await RefusedAsync(AiFeatureIds.PeriodSummaryId)).Code.ShouldBe(TrackerDomainErrorCodes.AiPlatformDisabled);

        await SetPlatformEnabledAsync(true);
        (await RefusedAsync(AiFeatureIds.PeriodSummaryId)).Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);
    }

    [Fact]
    public async Task A_Workspace_Whose_Owner_Has_Not_Turned_AI_On_Should_Be_Refused_As_Off()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);

        var refused = await RefusedAsync(AiFeatureIds.PeriodSummaryId);

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiDisabled);
        refused.Data["reason"].ShouldBe("off");

        await SetPolicyAsync(enabled: false);
        (await RefusedAsync(AiFeatureIds.AskId)).Data["reason"].ShouldBe("off");
    }

    [Fact]
    public async Task A_Changed_Disclosure_Should_Be_Refused_Until_The_Owner_Consents_Again()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync();
        AiOptions.Disclosure.Version = 1;

        var refused = await RefusedAsync(AiFeatureIds.PeriodSummaryId);

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiDisabled);
        refused.Data["reason"].ShouldBe("reconsent");

        await SetPolicyAsync();
        await AllowedAsync(AiFeatureIds.PeriodSummaryId);
    }

    [Fact]
    public async Task A_Client_Only_Member_Should_Be_Refused_Every_Feature()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await AddProjectMembershipAsync(CallerId, ProjectMemberRoles.Client);
        await AddProjectMembershipAsync(CallerId, "client");
        await SetPolicyAsync();

        var caller = await CallerAsync();
        caller.IsClientOnly.ShouldBeTrue();
        foreach (var feature in AiFeatureIds.All)
        {
            var refused = await RefusedAsync(feature.Id, caller);
            refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
            refused.Data["reason"].ShouldBe(AiFeatureReasons.Role);
        }
    }

    [Fact]
    public async Task A_Member_Who_Is_A_Client_Of_Some_Projects_But_Works_On_Others_Should_Not_Be_Client_Only()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await AddProjectMembershipAsync(CallerId, ProjectMemberRoles.Client);
        await AddProjectMembershipAsync(CallerId, ProjectMemberRoles.Worker);
        await SetPolicyAsync();

        var caller = await CallerAsync();

        caller.IsClientOnly.ShouldBeFalse();
        caller.HasWorkerProjectMembership.ShouldBeTrue();
        await AllowedAsync(AiFeatureIds.TaskDraftsId, caller);
        await AllowedAsync(AiFeatureIds.MemberDaySummaryId, caller);
    }

    [Fact]
    public async Task A_Manager_Should_Be_Refused_When_The_Owner_Does_Not_Allow_Managers()
    {
        await SignInAsAsync(WorkspaceRoles.Manager);
        await SetPolicyAsync(managers: false);

        var refused = await RefusedAsync(AiFeatureIds.ReportExplanationId);

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
        refused.Data["reason"].ShouldBe(AiFeatureReasons.Role);

        await SetPolicyAsync(managers: true);
        await AllowedAsync(AiFeatureIds.ReportExplanationId);
    }

    [Fact]
    public async Task A_Member_Should_Be_Refused_When_Self_Service_Is_Off()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await SetPolicyAsync(selfService: false);

        var refused = await RefusedAsync(AiFeatureIds.MemberDaySummaryId);

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
        refused.Data["reason"].ShouldBe(AiFeatureReasons.SelfServiceOff);
    }

    [Fact]
    public async Task A_Member_Should_Never_Get_Report_Explanations_Or_Team_Scopes()
    {
        await SignInAsAsync(WorkspaceRoles.Member);
        await SetPolicyAsync();
        var caller = await CallerAsync();

        var refused = await RefusedAsync(AiFeatureIds.ReportExplanationId, caller);
        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
        refused.Data["reason"].ShouldBe(AiFeatureReasons.Role);

        // Even when the test authorization grants every permission, a member asks about themselves only.
        caller.CanViewAll.ShouldBeTrue();
        caller.CanSeeTeam.ShouldBeFalse();
        _guard.Evaluate(caller, AiFeatureIds.PeriodSummary).Scopes.ShouldBe(new[] { AiFeatureScopes.Self });
        _guard.Evaluate(caller, AiFeatureIds.MemberDaySummary).Scopes.ShouldBe(new[] { AiFeatureScopes.Self });
        await AllowedAsync(AiFeatureIds.PeriodSummaryId, caller);
        await AllowedAsync(AiFeatureIds.NoteSuggestionsId, caller);

        // A member without a project they work on has nothing to draft tasks for.
        (await Should.ThrowAsync<AbpAuthorizationException>(() => WithUnitOfWorkAsync(() =>
            _guard.EnsureAsync(caller, AiFeatureIds.TaskDraftsId)))).ShouldNotBeNull();
    }

    [Fact]
    public async Task Owners_And_Managers_Should_See_The_Team_Only_With_ViewAll_And_Reporting()
    {
        await SignInAsAsync(WorkspaceRoles.Manager);
        await SetPolicyAsync();
        var caller = await CallerAsync();

        _guard.Evaluate(caller, AiFeatureIds.PeriodSummary).Scopes.ShouldBe(new[] { AiFeatureScopes.Team, AiFeatureScopes.Self });
        _guard.Evaluate(caller, AiFeatureIds.DataQualityReview).Scopes.ShouldBe(new[] { AiFeatureScopes.Team, AiFeatureScopes.Self });
        _guard.Evaluate(caller, AiFeatureIds.MemberDaySummary).Scopes.ShouldBe(new[] { AiFeatureScopes.Member, AiFeatureScopes.Self });
        _guard.Evaluate(caller, AiFeatureIds.Ask).Scopes.ShouldBeEmpty();

        var withoutReporting = caller with { CanReport = false };
        _guard.Evaluate(withoutReporting, AiFeatureIds.PeriodSummary).Scopes.ShouldBe(new[] { AiFeatureScopes.Self });
        _guard.Evaluate(withoutReporting, AiFeatureIds.ReportExplanation).ShouldBe(AiFeatureAccess.Refused(AiFeatureReasons.Permission));
        await Should.ThrowAsync<AbpAuthorizationException>(() => WithUnitOfWorkAsync(() =>
            _guard.EnsureAsync(withoutReporting, AiFeatureIds.ReportExplanationId)));

        var withoutProjectsEdit = caller with { CanEditProjects = false };
        _guard.Evaluate(withoutProjectsEdit, AiFeatureIds.TaskDrafts).Reason.ShouldBe(AiFeatureReasons.Permission);
        Provider.Client.Calls.Count.ShouldBe(0);
    }

    [Fact]
    public async Task A_Caller_Who_Is_Not_A_Member_Should_Be_Refused_For_Their_Role()
    {
        await SetPolicyAsync();

        var caller = await CallerAsync();

        caller.Role.ShouldBeNull();
        (await RefusedAsync(AiFeatureIds.PeriodSummaryId, caller)).Data["reason"].ShouldBe(AiFeatureReasons.Role);
    }

    [Fact]
    public async Task An_Unknown_Feature_Should_Be_A_Programming_Error()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        var caller = await CallerAsync();

        await Should.ThrowAsync<ArgumentException>(() => _guard.EnsureAsync(caller, "summarize-everything"));
    }

    [Fact]
    public async Task The_Caller_Context_Should_Carry_The_Zone_The_Plan_And_The_Policy()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await SetPolicyAsync(titles: true);
        await SetSubscriptionAsync(Subscription.ActiveStatus, planName: "Business");
        var factory = GetRequiredService<AiCallerContextFactory>();

        var caller = await WithUnitOfWorkAsync(() => factory.CreateAsync("Asia/Dhaka"));

        caller.WorkspaceId.ShouldBe(WorkspaceId);
        caller.UserId.ShouldBe(CallerId);
        caller.IsOwner.ShouldBeTrue();
        caller.TimeZone.ShouldBe("Asia/Dhaka");
        caller.PlanName.ShouldBe("Business");
        caller.Policy.AllowWindowTitles.ShouldBeTrue();
        caller.IsWorkspaceActive.ShouldBeTrue();
        caller.DisclosureVersion.ShouldBe("1.0");

        // An unknown zone falls back to the caller's reporting zone, else UTC.
        (await WithUnitOfWorkAsync(() => factory.CreateAsync("Mars/Olympus"))).TimeZone.ShouldBe("UTC");
        await WithUnitOfWorkAsync(() => GetRequiredService<Settings.WorkspaceSettingsManager>().SetDefaultTimeZoneAsync("Europe/London"));
        (await WithUnitOfWorkAsync(() => factory.CreateAsync(null))).TimeZone.ShouldBe("Europe/London");
    }

    [Fact]
    public async Task A_Workspace_Without_A_Subscription_Should_Use_The_Default_Plan()
    {
        await SignInAsAsync(WorkspaceRoles.Member);

        (await CallerAsync()).PlanName.ShouldBe(AiCallerContext.DefaultPlanName);
        AiOptions.Quotas.For(AiCallerContext.DefaultPlanName).ShouldBe(AiOptions.Quotas.Default);
        Provider.Client.Calls.ShouldBeEmpty();
    }
}
