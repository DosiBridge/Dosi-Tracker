using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>"Explain this day" for an owner: Ayesha's day (M1) and their own.</summary>
public abstract class MemberDaySummaryTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static AiMemberDaySummaryInput AyeshasDay => new() { UserId = AyeshaId, Date = DayText, TimeZone = "UTC" };

    private static string Answer(string text) =>
        Json(new { headline = text, summary = text, highlights = Array.Empty<object>(), caveats = Array.Empty<string>() });

    [Fact]
    public async Task The_Bundle_Totals_Should_Equal_The_Reports_For_The_Same_Day_And_Zone()
    {
        await ReadyAsync();

        var (scope, bundle) = await BundleAsync(Feature<MemberDaySummaryFeature>(), AyeshasDay);

        scope.Audience.ShouldBe(AiUsageRecord.MemberAudience);
        scope.SubjectUserId.ShouldBe(AyeshaId);
        var input = Input(AyeshaId);
        var summary = await ReadAsync(() => Reporting.GetSummaryAsync(input));
        var apps = await ReadAsync(() => Reporting.GetAppUsageAsync(input));
        var hourly = await ReadAsync(() => Reporting.GetHourlyAsync(input));

        Total(bundle, AiPeriodFacts.Tracked).ShouldBe(DisplayFormat.Duration(summary.TotalTrackedMinutes));
        Total(bundle, AiPeriodFacts.Tracked).ShouldBe("1 h 45 min");
        Total(bundle, AiPeriodFacts.Idle).ShouldBe(DisplayFormat.Duration(summary.TotalIdleMinutes));
        Total(bundle, AiPeriodFacts.Activity).ShouldBe(DisplayFormat.Percent(summary.AverageProductivity));
        Total(bundle, AiDayFacts.FirstStart).ShouldBe("10:00");
        Total(bundle, AiDayFacts.LastEnd).ShouldBe("12:45");
        Total(bundle, AiDayFacts.LongestSpan).ShouldBe("1 h");
        Total(bundle, AiDayFacts.Breaks).ShouldBe("1");
        Total(bundle, AiDayFacts.BreakTime).ShouldBe("1 h");

        var appFacts = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.AppsList)
            .ToDictionary(a => bundle.Labels[AiFeatureOutput.Value(a, "app")!], a => FactOf(bundle, a, "tracked"));
        appFacts.Count.ShouldBe(apps.Count);
        foreach (var app in apps)
        {
            appFacts[app.DisplayName].ShouldBe(DisplayFormat.Duration(app.TrackedMinutes));
        }

        var total = hourly.Sum(h => h.TrackedMinutes);
        var parts = bundle.Lists[AiDayFacts.DayPartsList]!.AsObject();
        FactOf(bundle, parts, "morning").ShouldBe(AiPeriodFacts.Share(hourly.Where(h => h.Hour < 12).Sum(h => h.TrackedMinutes), total));
        FactOf(bundle, parts, "afternoon").ShouldBe(AiPeriodFacts.Share(hourly.Where(h => h.Hour is >= 12 and < 17).Sum(h => h.TrackedMinutes), total));
        FactOf(bundle, parts, "evening").ShouldBe("0%");

        AiFeatureOutput.ListItems(bundle, AiPeriodFacts.ProjectsList).Select(p => bundle.Labels[AiFeatureOutput.Value(p, "project")!])
            .ShouldBe(new[] { "Client portal", "Website" });
        bundle.Facts.Keys.Where(k => bundle.FactSubjects.TryGetValue(k, out var s) && s == "M1").ShouldNotBeEmpty();
    }

    [Fact]
    public async Task What_Is_Sent_Should_Hold_Handles_And_Facts_And_Titles_And_Notes_Only_When_Allowed()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer("{M1} tracked {F1}."));

        await Ai.CreateMemberDaySummaryAsync(AyeshasDay);

        var off = Model.Calls.Last();
        ShouldHoldNothingPrivate(off);
        off.AllText.ShouldContain("M1");
        off.AllText.ShouldContain("1 h 45 min");
        off.AllText.ShouldNotContain(SalaryTitle);
        off.AllText.ShouldNotContain(SecretNote);

        await SetPolicyAsync(titles: true);
        Model.EnqueueJson(Answer("{M1} tracked {F1}."));
        await Ai.CreateMemberDaySummaryAsync(AyeshasDay);

        var titles = Model.Calls.Last();
        ShouldHoldNothingPrivate(titles);
        titles.AllText.ShouldContain(SalaryTitle);
        titles.AllText.ShouldNotContain(SecretNote);

        await SetPolicyAsync(notes: true);
        Model.EnqueueJson(Answer("{M1} tracked {F1}."));
        var withNotes = await Ai.CreateMemberDaySummaryAsync(AyeshasDay);

        var notes = Model.Calls.Last();
        ShouldHoldNothingPrivate(notes);
        notes.AllText.ShouldContain(SecretNote);
        notes.AllText.ShouldNotContain(SalaryTitle);
        withNotes.Meta.DataCategories.ShouldContain(AiDataCategories.TaskNotes);
        withNotes.Meta.DataCategories.ShouldNotContain(AiDataCategories.WindowTitles);
        Model.Calls.Count.ShouldBe(3);
    }

    [Fact]
    public async Task The_Answer_Should_Be_Rendered_With_The_Members_Name_And_The_Value()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer("{M1} tracked {F1}."));

        var result = await Ai.CreateMemberDaySummaryAsync(AyeshasDay);

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Headline.ShouldBe(AyeshaName + " tracked 1 h 45 min.");
        result.Summary.ShouldBe(AyeshaName + " tracked 1 h 45 min.");
        var row = (await RowsAsync()).ShouldHaveSingleItem();
        row.Audience.ShouldBe(AiUsageRecord.MemberAudience);
        row.SubjectUserId.ShouldBe(AyeshaId);
    }

    [Fact]
    public async Task Digits_In_The_Answer_Should_Cost_A_Repair_Then_Give_The_Basic_Summary()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer("{M1} tracked 5 hours.")).EnqueueJson(Answer("{M1} tracked 105 minutes."));

        var result = await Ai.CreateMemberDaySummaryAsync(AyeshasDay);

        Model.Calls.Count.ShouldBe(2);
        result.Meta.Status.ShouldBe(AiResultStatuses.Fallback);
        var today = DateTime.UtcNow.Date;
        result.Headline.ShouldBe($"{AyeshaName} tracked 1 h 45 min on {DisplayFormat.Day(Day, DisplayFormat.NeedsYear(Day, Day, today))}.");
        result.Summary.ShouldStartWith("The day ran from 10:00 to 12:45. Most time went to Client portal in ");
        result.Highlights.ShouldHaveSingleItem().Text.ShouldBe("The longest continuous stretch was 1 h, from 10:00 to 11:00.");
    }

    [Fact]
    public async Task The_Callers_Own_Day_Should_Be_Addressed_To_Them()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer("{ME} tracked {F1}."));

        var result = await Ai.CreateMemberDaySummaryAsync(new AiMemberDaySummaryInput { Date = DayText, TimeZone = "UTC" });

        result.Headline.ShouldBe("You tracked 50 min.");
        (await RowsAsync()).ShouldHaveSingleItem().Audience.ShouldBe(AiUsageRecord.SelfAudience);
    }

    [Fact]
    public async Task An_Empty_Day_Should_Be_Answered_Without_Calling_The_Model()
    {
        await ReadyAsync();

        var result = await Ai.CreateMemberDaySummaryAsync(new AiMemberDaySummaryInput { UserId = AyeshaId, Date = "2020-01-01", TimeZone = "UTC" });

        result.Meta.Status.ShouldBe(AiResultStatuses.NoData);
        result.Headline.ShouldBe(MemberDaySummaryFeature.NoDataHeadline);
        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Day_Should_Be_Read_In_The_Requested_Zone()
    {
        await ReadyAsync();

        // In Kathmandu (UTC+05:45) the day starts at 18:15 UTC the day before, so Ayesha's 10:00 UTC block starts at 15:45.
        var (_, bundle) = await BundleAsync(Feature<MemberDaySummaryFeature>(),
            new AiMemberDaySummaryInput { UserId = AyeshaId, Date = DayText, TimeZone = "Asia/Kathmandu" }, zone: "Asia/Kathmandu");

        Total(bundle, AiDayFacts.FirstStart).ShouldBe("15:45");
        Total(bundle, AiPeriodFacts.TimeZone).ShouldBe("Asia/Kathmandu (UTC+05:45)");
    }
}

/// <summary>A worker: their own day only.</summary>
public abstract class MemberDaySummaryWorkerTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task Another_Members_Day_Should_Be_Refused()
    {
        await ReadyAsync();

        await Should.ThrowAsync<AbpAuthorizationException>(() =>
            Ai.CreateMemberDaySummaryAsync(new AiMemberDaySummaryInput { UserId = AyeshaId, Date = DayText }));

        Model.Calls.ShouldBeEmpty();
        (await RowsAsync()).ShouldBeEmpty();
    }
}
