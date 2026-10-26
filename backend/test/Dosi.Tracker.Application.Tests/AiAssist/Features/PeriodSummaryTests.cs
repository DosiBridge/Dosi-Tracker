using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>Period summaries for an owner: the team, a project and their own week.</summary>
public abstract class PeriodSummaryTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static AiPeriodSummaryInput Team(Guid? projectId = null) =>
        new() { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Team, TimeZone = "UTC", ProjectId = projectId };

    [Fact]
    public async Task A_Team_Summary_Should_Be_Written_From_Handles_And_Facts_And_Rendered_With_Real_Names()
    {
        await ReadyAsync();
        Model.EnqueueJson(Json(new
        {
            headline = "The team tracked {F1}.",
            insights = new[] { new { kind = "distribution", text = "{M1} tracked the larger share, mostly on {P1}.", factRefs = new[] { "F1" }, routeId = "team" } },
            nextSteps = new[] { new { label = "Open the weekly report", routeId = "reports.weekly" } },
            caveats = Array.Empty<string>()
        }));

        var result = await Ai.CreatePeriodSummaryAsync(Team());

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Meta.PromptVersion.ShouldBe("period-summary@1");
        result.Meta.DataCategories.ShouldContain(AiDataCategories.ProjectNames);
        result.Meta.DataCategories.ShouldContain(AiDataCategories.AppNames);
        result.Headline.ShouldBe("The team tracked 2 h 35 min.");
        var insight = result.Insights.ShouldHaveSingleItem();
        insight.Text.ShouldBe(AyeshaName + " tracked the larger share, mostly on Client portal.");
        insight.RouteId.ShouldBe("team");
        var fact = insight.Facts.ShouldHaveSingleItem();
        (fact.Id, fact.Label, fact.Display).ShouldBe(("F1", "tracked time", "2 h 35 min"));
        result.NextSteps.ShouldHaveSingleItem().RouteId.ShouldBe("reports.weekly");

        var call = Model.Calls.ShouldHaveSingleItem();
        ShouldHoldNothingPrivate(call);
        call.AllText.ShouldNotContain(SalaryTitle);
        call.AllText.ShouldNotContain(SecretNote);
        call.AllText.ShouldContain("\"member\":\"M1\"");
        call.AllText.ShouldContain("\"member\":\"ME\"");
        (await RowsAsync()).ShouldHaveSingleItem().Audience.ShouldBe(AiUsageRecord.TeamAudience);
    }

    [Fact]
    public async Task Members_Should_Carry_Their_Tracked_Time_Only()
    {
        await ReadyAsync();

        var (scope, bundle) = await BundleAsync(Feature<PeriodSummaryFeature>(), Team());

        scope.Audience.ShouldBe(AiUsageRecord.TeamAudience);
        var members = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.MembersList);
        members.Select(m => AiFeatureOutput.Value(m, "member")).ShouldBe(new[] { "M1", "ME" });
        members.ShouldAllBe(m => m.Count == 2 && m.ContainsKey("tracked"));
        FactOf(bundle, members[0], "tracked").ShouldBe("1 h 45 min");
        bundle.Resolution["M1"].DisplayName.ShouldBe(AyeshaName);
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("2 h 35 min");
        Total(bundle, AiPeriodFacts.PreviousTracked).ShouldBe("0 min");
        Total(bundle, PeriodSummaryFeature.MembersWithoutTime).ShouldBe("0");
        AiFeatureOutput.ListItems(bundle, AiPeriodFacts.ProjectsList).Select(p => bundle.Labels[AiFeatureOutput.Value(p, "project")!])
            .ShouldBe(new[] { "Client portal", "Website" });
        bundle.Routes.ShouldContain("team");
    }

    [Fact]
    public async Task At_Most_Twelve_Members_Should_Get_A_Handle_And_The_Rest_Be_Folded()
    {
        await ReadyAsync();
        for (var i = 0; i < 14; i++)
        {
            var member = await CreateMemberAsync("member" + i, WorkspaceRoles.Member, name: "Member " + i);
            await InsertBlockAsync(member, WebsiteId, Day.AddHours(15), 5 + i);
        }

        var (_, bundle) = await BundleAsync(Feature<PeriodSummaryFeature>(), Team());

        var handles = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.MembersList).Select(m => AiFeatureOutput.Value(m, "member")).ToList();
        handles.Count.ShouldBe(13);
        handles.ShouldContain("ME");
        handles.Where(h => h != "ME").ShouldBe(Enumerable.Range(1, 12).Select(n => "M" + n), ignoreOrder: true);
        bundle.Resolution.Keys.ShouldNotContain("M13");
        // Ayesha and 14 more: 12 handles, 3 folded.
        bundle.Facts[AiFeatureOutput.ListValue(bundle, AiPeriodFacts.OtherMembersList, "count")!].Display.ShouldBe("3");
    }

    [Fact]
    public async Task A_Range_Over_Thirty_One_Days_Should_Be_Refused_Before_Anything_Is_Read()
    {
        await ReadyAsync();

        var refused = await Should.ThrowAsync<BusinessException>(() =>
            Ai.CreatePeriodSummaryAsync(new AiPeriodSummaryInput { From = Day.AddDays(-40), To = Day, Scope = AiFeatureScopes.Team }));

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiRangeTooLarge);
        refused.Data["maxDays"].ShouldBe(AiAssistConsts.MaxPeriodDays);
        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Project_Pulse_Should_Need_A_Visible_Project_And_Carry_Its_Task_Counts()
    {
        await ReadyAsync();

        var (scope, bundle) = await BundleAsync(Feature<PeriodSummaryFeature>(), Team(WebsiteId));

        scope.Audience.ShouldBe(AiUsageRecord.ProjectAudience);
        scope.ProjectId.ShouldBe(WebsiteId);
        bundle.Labels["P1"].ShouldBe("Website");
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("1 h 15 min");
        Total(bundle, PeriodSummaryFeature.OpenTasks).ShouldBe("1");
        Total(bundle, PeriodSummaryFeature.CompletedTasks).ShouldBe("0");
        bundle.Lists.ContainsKey(AiPeriodFacts.ProjectsList).ShouldBeFalse();

        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreatePeriodSummaryAsync(Team(Guid.NewGuid())));

        var elsewhere = await ServiceProvider.CreateWorkspaceAsync("Elsewhere Co");
        var foreign = Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(elsewhere))
            {
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(foreign, elsewhere, "Not yours"), autoSave: true);
            }
        });
        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreatePeriodSummaryAsync(Team(foreign)));
        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Owners_Own_Week_Should_Hold_No_Members()
    {
        await ReadyAsync();

        var (scope, bundle) = await BundleAsync(Feature<PeriodSummaryFeature>(),
            new AiPeriodSummaryInput { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Self });

        scope.Audience.ShouldBe(AiUsageRecord.SelfAudience);
        scope.SubjectUserId.ShouldBe(CallerId);
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("50 min");
        bundle.Lists.ContainsKey(AiPeriodFacts.MembersList).ShouldBeFalse();
        bundle.Resolution.Keys.ShouldBe(new[] { "ME" });
    }

    [Fact]
    public async Task A_Provider_Failure_Should_Be_Answered_With_The_Basic_Summary()
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.ServerError, 500));

        var result = await Ai.CreatePeriodSummaryAsync(Team());

        result.Meta.Status.ShouldBe(AiResultStatuses.Fallback);
        result.Meta.Model.ShouldBeNull();
        result.Headline.ShouldStartWith("The team tracked 2 h 35 min in ");
        result.Insights.Select(i => i.Text).ShouldContain("Most tracked time went to Client portal: 1 h 20 min, 52% of the total.");
        result.Insights.Select(i => i.Text).ShouldContain(t => t.StartsWith("The app most in focus was "));
        result.Insights.Select(i => i.Text).ShouldContain("Change from the previous period of the same length (0 min): +2 h 35 min.");
        result.Insights.ShouldAllBe(i => i.Facts.Count > 0);
    }

    [Fact]
    public async Task An_Empty_Period_Should_Be_Answered_Without_Calling_The_Model()
    {
        await ReadyAsync();
        var from = new DateTime(2020, 3, 2, 0, 0, 0, DateTimeKind.Utc);

        var result = await Ai.CreatePeriodSummaryAsync(new AiPeriodSummaryInput { From = from, To = from.AddDays(7), Scope = AiFeatureScopes.Team });

        result.Meta.Status.ShouldBe(AiResultStatuses.NoData);
        result.Headline.ShouldBe(PeriodSummaryFeature.NoDataHeadline);
        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Answer_With_A_Digit_Should_Be_Repaired_Once()
    {
        await ReadyAsync();
        var bad = Json(new { headline = "The team tracked 2 hours.", insights = Array.Empty<object>(), nextSteps = Array.Empty<object>(), caveats = Array.Empty<string>() });
        var good = Json(new { headline = "The team tracked {F1}.", insights = Array.Empty<object>(), nextSteps = Array.Empty<object>(), caveats = Array.Empty<string>() });
        Model.EnqueueJson(bad).EnqueueJson(good);

        var result = await Ai.CreatePeriodSummaryAsync(Team());

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Headline.ShouldBe("The team tracked 2 h 35 min.");
        Model.Calls.Count.ShouldBe(2);
        Model.Calls[1].Messages.Last().Text.ShouldContain("headline: contains a digit");
    }

    [Fact]
    public void The_Prompt_Should_Use_The_Supported_Schema_Subset()
    {
        Prompts.PeriodSummaryPrompt.Spec.Version.ShouldBe("period-summary@1");
        Prompts.AiPromptSpec.SchemaViolations(Prompts.PeriodSummaryPrompt.Spec.Schema).ShouldBeEmpty();
    }
}

/// <summary>A worker (a member's permissions): their own week only; the team is refused, never narrowed.</summary>
public abstract class PeriodSummaryWorkerTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(
            TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage,
            TrackerPermissions.Projects.Edit);
    }

    [Fact]
    public async Task The_Team_Should_Be_Refused_And_Their_Own_Week_Answered()
    {
        await ReadyAsync();

        await Should.ThrowAsync<AbpAuthorizationException>(() =>
            Ai.CreatePeriodSummaryAsync(new AiPeriodSummaryInput { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Team }));
        Model.Calls.ShouldBeEmpty();

        Model.EnqueueJson(Json(new { headline = "{ME} tracked {F1}.", insights = Array.Empty<object>(), nextSteps = Array.Empty<object>(), caveats = Array.Empty<string>() }));
        var mine = await Ai.CreatePeriodSummaryAsync(new AiPeriodSummaryInput { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Self, TimeZone = "UTC" });

        mine.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        mine.Headline.ShouldBe("You tracked 50 min.");
        ShouldHoldNothingPrivate(Model.Calls.ShouldHaveSingleItem());
    }

    [Fact]
    public async Task A_Project_They_Do_Not_Belong_To_Should_Be_Not_Found()
    {
        await ReadyAsync();
        var hidden = await AddProjectMembershipAsync(AyeshaId, ProjectMemberRoles.Worker);

        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreatePeriodSummaryAsync(
            new AiPeriodSummaryInput { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Self, ProjectId = hidden }));
        Model.Calls.ShouldBeEmpty();
    }
}

/// <summary>A manager without Tracker.Reporting does not see the team either.</summary>
public abstract class PeriodSummaryManagerWithoutReportingTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Manager;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Reporting.Default);
    }

    [Fact]
    public async Task The_Team_Should_Be_Refused()
    {
        await ReadyAsync();

        var refused = await Should.ThrowAsync<AbpAuthorizationException>(() =>
            Ai.CreatePeriodSummaryAsync(new AiPeriodSummaryInput { From = Day, To = Day.AddDays(1), Scope = AiFeatureScopes.Team }));

        refused.Message.ShouldBe(Read.AiScopeGuard.TeamScopeMessage);
        Model.Calls.ShouldBeEmpty();
    }
}
