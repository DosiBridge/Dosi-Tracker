using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Users;
using Volo.Abp.Validation;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>The reads AI features made, in order (by the read layer's method names).</summary>
public sealed class AiReadCalls
{
    private readonly List<string> _calls = new();

    public IReadOnlyList<string> All
    {
        get
        {
            lock (_calls)
            {
                return _calls.ToList();
            }
        }
    }

    public void Add(string call)
    {
        lock (_calls)
        {
            _calls.Add(call);
        }
    }

    public void Clear()
    {
        lock (_calls)
        {
            _calls.Clear();
        }
    }
}

/// <summary>The real read layer, recording which reads a feature makes (registered only by the tests that use it).</summary>
[DisableConventionalRegistration]
public class RecordingAiReadLayer : AiReadLayer
{
    private readonly AiReadCalls _calls;

    public RecordingAiReadLayer(
        IReportingAppService reporting,
        IActivityAppService activities,
        ITeamAppService team,
        IProjectAppService projects,
        IProjectTaskAppService tasks,
        ActivityNoteRules noteRules,
        ICurrentUser currentUser,
        ICurrentWorkspace currentWorkspace,
        AiReadCalls calls)
        : base(reporting, activities, team, projects, tasks, noteRules, currentUser, currentWorkspace)
    {
        _calls = calls;
    }

    public override Task<AiSummary> SummaryAsync(AiCallerContext caller, AiQuery query) => Record(nameof(SummaryAsync), base.SummaryAsync(caller, query));
    public override Task<IReadOnlyList<AiDailyRow>> DailyAsync(AiCallerContext caller, AiQuery query) => Record(nameof(DailyAsync), base.DailyAsync(caller, query));
    public override Task<IReadOnlyList<AiMemberDayRow>> UserDailyAsync(AiCallerContext caller, AiQuery query) => Record(nameof(UserDailyAsync), base.UserDailyAsync(caller, query));
    public override Task<IReadOnlyList<AiAppUsageRow>> AppUsageAsync(AiCallerContext caller, AiQuery query) => Record(nameof(AppUsageAsync), base.AppUsageAsync(caller, query));
    public override Task<AiAttendance> AttendanceAsync(AiCallerContext caller, AiQuery query) => Record(nameof(AttendanceAsync), base.AttendanceAsync(caller, query));
    public override Task<IReadOnlyList<AiHourlyRow>> HourlyAsync(AiCallerContext caller, AiQuery query) => Record(nameof(HourlyAsync), base.HourlyAsync(caller, query));
    public override Task<IReadOnlyList<AiDeviceRow>> DevicesAsync(AiCallerContext caller, AiQuery query) => Record(nameof(DevicesAsync), base.DevicesAsync(caller, query));
    public override Task<IReadOnlyList<AiDataQualityRow>> DataQualityAsync(AiCallerContext caller, AiQuery query) => Record(nameof(DataQualityAsync), base.DataQualityAsync(caller, query));
    public override Task<IReadOnlyList<AiRosterEntry>> RosterAsync(AiCallerContext caller) => Record(nameof(RosterAsync), base.RosterAsync(caller));

    private Task<T> Record<T>(string name, Task<T> read)
    {
        _calls.Add(name);
        return read;
    }
}

/// <summary>"Explain this report" for an owner: what each report reads, what is never sent, roll-ups and the fallback.</summary>
public abstract class ReportExplanationTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.AddSingleton<AiReadCalls>();
        services.Replace(ServiceDescriptor.Transient<IAiReadLayer, RecordingAiReadLayer>());
    }

    private AiReadCalls Calls => GetRequiredService<AiReadCalls>();

    private static AiReportExplanationInput Report(string slug, int days = 1) =>
        new() { ReportSlug = slug, From = Day.AddDays(1 - days), To = Day.AddDays(1), TimeZone = "UTC" };

    private static string Answer(string summary) => Json(new
    {
        summary,
        keyFindings = Array.Empty<object>(),
        caveats = Array.Empty<string>(),
        questionsToExplore = new[] { "Which projects took the most time?" }
    });

    public static TheoryData<string, string[]> SlugReads => new()
    {
        { "time-activity", new[] { "SummaryAsync", "SummaryAsync", "DailyAsync" } },
        { "weekly", new[] { "SummaryAsync", "SummaryAsync", "DailyAsync", "UserDailyAsync" } },
        { "projects", new[] { "SummaryAsync", "SummaryAsync" } },
        { "productivity", new[] { "SummaryAsync", "SummaryAsync", "HourlyAsync" } },
        { "apps-urls", new[] { "SummaryAsync", "SummaryAsync", "AppUsageAsync" } },
        { "attendance", new[] { "SummaryAsync", "SummaryAsync", "AttendanceAsync", "RosterAsync" } },
        { "devices", new[] { "SummaryAsync", "SummaryAsync", "DevicesAsync", "RosterAsync" } },
        { "work-report", new[] { "SummaryAsync", "SummaryAsync", "DailyAsync" } },
        { "analytics", new[] { "SummaryAsync", "SummaryAsync", "DailyAsync", "HourlyAsync" } }
    };

    [Theory]
    [MemberData(nameof(SlugReads))]
    public async Task Each_Report_Should_Read_What_It_Explains(string slug, string[] reads)
    {
        await ReadyAsync();
        Calls.Clear();

        var (scope, bundle) = await BundleAsync(Feature<ReportExplanationFeature>(), Report(slug));

        Calls.All.ShouldBe(reads);
        scope.ReportSlug.ShouldBe(slug);
        scope.Audience.ShouldBe(AiUsageRecord.TeamAudience);
        bundle.Lists[ReportExplanationFeature.ReportList]!.GetValue<string>().ShouldBe(slug);
        bundle.Routes.ShouldContain("reports." + slug);
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("2 h 35 min");
        bundle.Lists.ContainsKey(AiReportFacts.MainListFor(slug)).ShouldBeTrue();
    }

    [Fact]
    public void Every_Explainable_Report_Should_Have_A_Data_Plan()
    {
        foreach (var slug in AiScopeGuard.ReportSlugs)
        {
            AiReportFacts.SourcesFor(slug).HasFlag(AiReportSources.Summary).ShouldBeTrue(slug);
        }

        Should.Throw<ArgumentException>(() => AiReportFacts.SourcesFor("payroll"));
    }

    [Theory]
    [InlineData("payroll")]
    [InlineData("Weekly")]
    [InlineData("salaries")]
    public async Task Payroll_And_Unknown_Reports_Should_Be_Refused(string slug)
    {
        await ReadyAsync();

        await Should.ThrowAsync<AbpValidationException>(() => Ai.CreateReportExplanationAsync(Report(slug)));

        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Devices_Report_Should_Send_Platforms_And_Agent_Versions_Only()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer("{M1} and {ME} tracked with the agent."));

        var result = await Ai.CreateReportExplanationAsync(Report("devices"));

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Meta.DataCategories.ShouldContain(AiDataCategories.AgentInfo);
        result.Summary.ShouldBe(AyeshaName + " and you tracked with the agent.");
        result.QuestionsToExplore.ShouldBe(new[] { "Which projects took the most time?" });
        var call = Model.Calls.ShouldHaveSingleItem();
        ShouldHoldNothingPrivate(call);
        call.AllText.ShouldContain("\"display\":\"windows\"");
        call.AllText.ShouldContain("\"display\":\"1.4.0\"");
        call.AllText.ShouldNotContain("Windows 11");
    }

    [Theory]
    [InlineData(7, AiPeriodFacts.DaysList)]
    [InlineData(60, AiPeriodFacts.WeeksList)]
    [InlineData(120, AiPeriodFacts.MonthsList)]
    public async Task Long_Ranges_Should_Be_Rolled_Up(int days, string list)
    {
        await ReadyAsync();

        var (_, bundle) = await BundleAsync(Feature<ReportExplanationFeature>(), Report("time-activity", days));

        bundle.Lists.ContainsKey(list).ShouldBeTrue();
        AiFeatureOutput.ListItems(bundle, list).Count.ShouldBeLessThanOrEqualTo(AiAssistConsts.MaxBundleDailyPoints);
        // The previous period is compared only up to 92 days.
        bundle.Lists[AiPeriodFacts.TotalsList]!.AsObject().ContainsKey(AiPeriodFacts.PreviousTracked).ShouldBe(days <= 92);
    }

    [Fact]
    public async Task A_Range_Over_A_Year_Should_Be_Refused()
    {
        await ReadyAsync();

        var refused = await Should.ThrowAsync<BusinessException>(() => Ai.CreateReportExplanationAsync(Report("weekly", 400)));

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiRangeTooLarge);
        refused.Data["maxDays"].ShouldBe(AiAssistConsts.MaxReportDays);
    }

    [Fact]
    public async Task A_Member_Filter_Should_Explain_That_Member_Only()
    {
        await ReadyAsync();

        var (scope, bundle) = await BundleAsync(Feature<ReportExplanationFeature>(),
            new AiReportExplanationInput { ReportSlug = "productivity", From = Day, To = Day.AddDays(1), UserId = AyeshaId, DayBasis = "member" });

        scope.Audience.ShouldBe(AiUsageRecord.MemberAudience);
        scope.SubjectUserId.ShouldBe(AyeshaId);
        scope.DayBasis.ShouldBe(ReportDayBasis.Member);
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("1 h 45 min");
        var member = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.MembersList).ShouldHaveSingleItem();
        AiFeatureOutput.Value(member, "member").ShouldBe("M1");
        FactOf(bundle, member, "activity").ShouldBe("70%");
    }

    [Fact]
    public async Task A_Provider_Failure_Should_List_The_Top_Rows_Of_The_Report()
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.RateLimited, 429));

        var result = await Ai.CreateReportExplanationAsync(Report("projects"));

        result.Meta.Status.ShouldBe(AiResultStatuses.Fallback);
        result.Summary.ShouldStartWith("The team tracked 2 h 35 min in ");
        result.KeyFindings.Select(f => f.Text).ShouldBe(new[]
        {
            "Client portal: 1 h 20 min, 52% of the tracked time.",
            "Website: 1 h 15 min, 48% of the tracked time."
        });
        result.KeyFindings[0].Facts.Select(f => f.Label).ShouldBe(new[] { "tracked time on the project (Client portal)", "share of the tracked time (Client portal)" });
    }

    [Theory]
    [InlineData("time-activity", "distribution", AyeshaName + " tracked 1 h 45 min.")]
    [InlineData("weekly", "distribution", AyeshaName + " tracked 1 h 45 min.")]
    [InlineData("productivity", "distribution", AyeshaName + " tracked 1 h 45 min.")]
    [InlineData("apps-urls", "apps", null)]
    [InlineData("attendance", "attendance", null)]
    [InlineData("devices", "devices", null)]
    [InlineData("work-report", "distribution", "Client portal: 1 h 20 min, 52% of the tracked time.")]
    [InlineData("analytics", "distribution", "Client portal: 1 h 20 min, 52% of the tracked time.")]
    public async Task Every_Report_Should_Have_A_Basic_Summary(string slug, string kind, string? first)
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.ServerError, 502));

        var result = await Ai.CreateReportExplanationAsync(Report(slug));

        result.Meta.Status.ShouldBe(AiResultStatuses.Fallback);
        result.Summary.ShouldStartWith("The team tracked 2 h 35 min in ");
        result.KeyFindings.Count.ShouldBeInRange(1, ReportExplanationFeature.MaxFallbackRows);
        result.KeyFindings.ShouldAllBe(f => f.Kind == kind && f.Facts.Count > 0);
        if (first != null)
        {
            result.KeyFindings[0].Text.ShouldBe(first);
        }

        result.KeyFindings.ShouldAllBe(f => !f.Text.Contains('{'));
    }

    [Fact]
    public async Task The_Basic_Summary_Of_One_Member_Or_One_Project_Should_Name_Them()
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.ServerError, 502))
            .EnqueueThrow(new AiProviderException(AiProviderErrorKind.ServerError, 502));

        var member = await Ai.CreateReportExplanationAsync(
            new AiReportExplanationInput { ReportSlug = "weekly", From = Day, To = Day.AddDays(1), UserId = AyeshaId, TimeZone = "UTC" });
        var project = await Ai.CreateReportExplanationAsync(
            new AiReportExplanationInput { ReportSlug = "time-activity", From = Day, To = Day.AddDays(1), ProjectId = WebsiteId, TimeZone = "UTC" });

        member.Summary.ShouldStartWith(AyeshaName + " tracked 1 h 45 min in ");
        project.Summary.ShouldStartWith("The team tracked 1 h 15 min on Website in ");
    }

    [Fact]
    public async Task The_Attendance_Report_Should_Explain_What_Absent_Means()
    {
        await ReadyAsync();

        var (_, bundle) = await BundleAsync(Feature<ReportExplanationFeature>(), Report("attendance"));

        bundle.Caveats.ShouldContain(AiReportFacts.AttendanceCaveat);
        var rows = AiFeatureOutput.ListItems(bundle, AiReportFacts.AttendanceList);
        rows.Select(r => AiFeatureOutput.Value(r, "member")).ShouldBe(new[] { "M1", "ME" });
        bundle.Resolution["M1"].DisplayName.ShouldBe(AyeshaName);
    }
}

/// <summary>Members never get report explanations.</summary>
public abstract class ReportExplanationMemberTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task A_Member_Should_Be_Refused_For_Their_Role()
    {
        await ReadyAsync();

        var refused = await Should.ThrowAsync<BusinessException>(() => Ai.CreateReportExplanationAsync(
            new AiReportExplanationInput { ReportSlug = "weekly", From = Day, To = Day.AddDays(1) }));

        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiFeatureDisabled);
        refused.Data["reason"].ShouldBe(AiFeatureReasons.Role);
        Model.Calls.ShouldBeEmpty();
    }
}
