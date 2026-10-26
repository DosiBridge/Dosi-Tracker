using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Shouldly;
using Volo.Abp.Json;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Reporting;

/// <summary>The reports as a caller who sees the whole team (the test authorization grants every permission).</summary>
public abstract class ReportingAppServiceTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private Task TrackAsync(Guid projectId, DateTime start, int minutes, int? productivity)
    {
        return InsertAsync(Block(projectId, start, minutes, productivity: productivity));
    }

    private Task TrackWithAppsAsync(Guid projectId, DateTime start, int minutes, string[] active, string[] running, Guid? userId = null)
    {
        return InsertAsync(Block(projectId, start, minutes, userId, configure: block =>
        {
            block.ActiveWindowsJson = StoredWindows(active.Select(a => (a, (int?)null)).ToArray());
            block.RunningProgramsJson = StoredWindows(running.Select(a => (a, (int?)null)).ToArray());
        }));
    }

    private Task TrackWithWindowsAsync(Guid projectId, DateTime start, int minutes, params (string App, int? Seconds)[] active)
    {
        return InsertAsync(Block(projectId, start, minutes, configure: block => block.ActiveWindowsJson = StoredWindows(active)));
    }

    // ---- Summary ----

    [Fact]
    public async Task Summary_Should_Use_Real_Durations_And_Duration_Weighted_Productivity()
    {
        var projectA = await SeedMyProjectAsync("A");
        var projectB = await SeedMyProjectAsync("B");

        // 30 min @ 100 and 10 min @ 60 on project A; 20 min @ 50 on project B.
        await TrackAsync(projectA, Day.AddHours(9), 30, 100);
        await TrackAsync(projectA, Day.AddHours(10), 10, 60);
        await TrackAsync(projectB, Day.AddHours(11), 20, 50);

        var summary = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));

        summary.TotalActivities.ShouldBe(3);
        summary.TotalTrackedMinutes.ShouldBe(60);
        summary.TotalMouseClicks.ShouldBe(30);
        summary.TotalKeyboardHits.ShouldBe(60);

        // Weighted: (30*100 + 10*60 + 20*50) / 60 = (3000 + 600 + 1000) / 60 = 76.67
        summary.AverageProductivity.ShouldBe(76.67);

        summary.PerProject.Count.ShouldBe(2);
        var rowA = summary.PerProject.Single(p => p.ProjectId == projectA);
        rowA.ActivityCount.ShouldBe(2);
        rowA.TrackedMinutes.ShouldBe(40);
        rowA.AverageProductivity.ShouldBe(90); // (30*100 + 10*60) / 40

        var rowB = summary.PerProject.Single(p => p.ProjectId == projectB);
        rowB.TrackedMinutes.ShouldBe(20);
        rowB.AverageProductivity.ShouldBe(50);

        summary.PerUser.Count.ShouldBe(1); // single test user
        summary.PerUser[0].TrackedMinutes.ShouldBe(60);
    }

    [Fact]
    public async Task Input_Totals_Should_Sum_Only_Measured_Blocks_And_Say_How_Many_Measured()
    {
        var projectId = await SeedMyProjectAsync();
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);                                // 10 clicks, 20 keys
        await InsertAsync(Block(projectId, Day.AddHours(10), 30, configure: b =>
        {
            b.MouseClicks = null; // e.g. a Wayland session: not measured
            b.KeyboardHits = null;
        }));
        await InsertAsync(Block(projectId, Day.AddHours(11), 30, configure: b =>
        {
            b.MouseClicks = 0;    // measured, no clicks
            b.KeyboardHits = null;
        }));

        var summary = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));

        summary.TotalActivities.ShouldBe(3);
        summary.TotalMouseClicks.ShouldBe(10);
        summary.MouseMeasuredActivities.ShouldBe(2);
        summary.TotalKeyboardHits.ShouldBe(20);
        summary.KeyboardMeasuredActivities.ShouldBe(1);

        var day = (await Reporting.GetDailySeriesAsync(Range(Day, Day.AddDays(1)))).ShouldHaveSingleItem();
        day.MouseClicks.ShouldBe(10);
        day.MouseMeasuredActivities.ShouldBe(2);
        day.KeyboardHits.ShouldBe(20);
        day.KeyboardMeasuredActivities.ShouldBe(1);
    }

    [Fact]
    public async Task Input_Totals_Of_A_Range_Without_Measured_Blocks_Should_Be_Recognisable_As_Unmeasured()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(Block(projectId, Day.AddHours(9), 30, productivity: null, configure: b =>
        {
            b.MouseClicks = null;
            b.KeyboardHits = null;
        }));

        var summary = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));

        summary.TotalActivities.ShouldBe(1);
        summary.TotalMouseClicks.ShouldBe(0);
        summary.MouseMeasuredActivities.ShouldBe(0); // so the 0 above reads as "not measured"
        summary.KeyboardMeasuredActivities.ShouldBe(0);
    }

    [Fact]
    public async Task Averages_Should_Skip_Blocks_Whose_Productivity_Was_Not_Measured()
    {
        var projectId = await SeedMyProjectAsync();
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);

        // A 60-minute block without input measurement (null): tracked time, but never averaged in as 0%.
        await TrackAsync(projectId, Day.AddHours(10), 60, null);

        var range = Range(Day, Day.AddDays(1));
        var summary = await Reporting.GetSummaryAsync(range);

        summary.TotalTrackedMinutes.ShouldBe(90);
        summary.AverageProductivity.ShouldBe(80);
        summary.PerUser.ShouldHaveSingleItem().AverageProductivity.ShouldBe(80);
        summary.PerProject.ShouldHaveSingleItem().AverageProductivity.ShouldBe(80);
        summary.PerUserProject.ShouldHaveSingleItem().AverageProductivity.ShouldBe(80);

        var series = await Reporting.GetDailySeriesAsync(range);
        series.ShouldHaveSingleItem().TrackedMinutes.ShouldBe(90);
        series[0].AverageProductivity.ShouldBe(80);
    }

    [Fact]
    public async Task Averages_Should_Be_Null_When_Nothing_Was_Measured()
    {
        var projectId = await SeedMyProjectAsync();
        await TrackAsync(projectId, Day.AddHours(9), 30, null);

        var range = Range(Day, Day.AddDays(1));
        var summary = await Reporting.GetSummaryAsync(range);

        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.AverageProductivity.ShouldBeNull();
        summary.PerUser.ShouldHaveSingleItem().AverageProductivity.ShouldBeNull();
        summary.PerProject.ShouldHaveSingleItem().AverageProductivity.ShouldBeNull();
        (await Reporting.GetDailySeriesAsync(range)).ShouldHaveSingleItem().AverageProductivity.ShouldBeNull();
    }

    [Fact]
    public async Task Summary_Should_Respect_Range_And_Project_Filters()
    {
        var projectA = await SeedMyProjectAsync("A");
        var projectB = await SeedMyProjectAsync("B");

        await TrackAsync(projectA, Day.AddHours(9), 30, 80);
        await TrackAsync(projectB, Day.AddHours(10), 15, 40);
        await TrackAsync(projectA, Day.AddDays(5), 60, 90); // outside range

        var input = Range(Day, Day.AddDays(1));
        input.ProjectId = projectA;
        var summary = await Reporting.GetSummaryAsync(input);

        summary.TotalActivities.ShouldBe(1);
        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.AverageProductivity.ShouldBe(80);
        summary.PerProject.Count.ShouldBe(1);
        summary.PerProject[0].ProjectId.ShouldBe(projectA);
    }

    [Fact]
    public async Task A_Team_Caller_Asking_For_Themselves_Should_Get_Only_Their_Own_Time()
    {
        // What a desktop agent of an owner or manager asks for its "Tracked today" card: UserId = the signed-in member.
        var projectId = await SeedMyProjectAsync();
        await AddMemberAsync(projectId, OtherUserId);
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);
        await InsertAsync(Block(projectId, Day.AddHours(9), 45, OtherUserId, productivity: 20));

        var mine = Range(Day, Day.AddDays(1), "UTC");
        mine.UserId = CurrentUserId;
        var summary = await Reporting.GetSummaryAsync(mine);

        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.AverageProductivity.ShouldBe(80);
        summary.PerUser.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
        (await Reporting.GetDailySeriesAsync(mine)).Sum(d => d.TrackedMinutes).ShouldBe(30);

        // Without the UserId the same caller gets the whole team.
        (await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1), "UTC"))).TotalTrackedMinutes.ShouldBe(75);
    }

    [Fact]
    public async Task Summary_Should_Be_Empty_For_A_Range_With_No_Activity()
    {
        var summary = await Reporting.GetSummaryAsync(Range(Day.AddYears(-1), Day.AddYears(-1).AddDays(1)));

        summary.TotalActivities.ShouldBe(0);
        summary.TotalTrackedMinutes.ShouldBe(0);
        summary.TotalIdleMinutes.ShouldBe(0);
        summary.AverageProductivity.ShouldBeNull();
        summary.PerUser.ShouldBeEmpty();
        summary.PerProject.ShouldBeEmpty();
        summary.PerUserProject.ShouldBeEmpty();
    }

    [Fact]
    public async Task Summary_Should_Name_Members_And_Projects_And_Sum_Measured_Idle_Time()
    {
        await CreateUserAsync(CurrentUserId, "me", "Ada", "Lovelace");
        await CreateUserAsync(OtherUserId, "bob");
        var website = await SeedProjectAsync("Website");
        var legacy = await SeedProjectAsync("Legacy", archived: true);
        var dropped = await SeedProjectAsync("Dropped", deleted: true);

        await InsertAsync(
            Block(website, Day.AddHours(9), 30, configure: b => b.IdleSeconds = 600),              // 10 idle minutes
            Block(legacy, Day.AddHours(10), 20, configure: b => b.IdleSeconds = 99_999),          // clamped to the block: 20
            Block(dropped, Day.AddHours(11), 15),                                                  // idle not measured: 0
            Block(website, Day.AddHours(9), 45, OtherUserId, configure: b => b.IdleSeconds = 90)); // 1.5

        var summary = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));

        summary.TotalIdleMinutes.ShouldBe(31.5);

        var me = summary.PerUser.Single(u => u.UserId == CurrentUserId);
        me.UserName.ShouldBe("me");
        me.Name.ShouldBe("Ada Lovelace");
        me.IdleMinutes.ShouldBe(30);
        var bob = summary.PerUser.Single(u => u.UserId == OtherUserId);
        bob.UserName.ShouldBe("bob");
        bob.Name.ShouldBeNull();
        bob.IdleMinutes.ShouldBe(1.5);

        var websiteRow = summary.PerProject.Single(p => p.ProjectId == website);
        websiteRow.ProjectTitle.ShouldBe("Website");
        websiteRow.IsArchived.ShouldBeFalse();
        websiteRow.IdleMinutes.ShouldBe(11.5);

        // Time on archived and deleted projects still counts, under its title, marked as no longer active.
        var legacyRow = summary.PerProject.Single(p => p.ProjectId == legacy);
        legacyRow.ProjectTitle.ShouldBe("Legacy");
        legacyRow.IsArchived.ShouldBeTrue();
        var droppedRow = summary.PerProject.Single(p => p.ProjectId == dropped);
        droppedRow.ProjectTitle.ShouldBe("Dropped");
        droppedRow.IsArchived.ShouldBeTrue();
        droppedRow.IdleMinutes.ShouldBe(0);
    }

    [Fact]
    public async Task Summary_Should_Leave_Names_Empty_For_Accounts_That_Do_Not_Exist()
    {
        var projectId = await SeedMyProjectAsync("Website");
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);

        var summary = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));

        var me = summary.PerUser.ShouldHaveSingleItem();
        me.UserName.ShouldBe(string.Empty);
        me.Name.ShouldBeNull();
        summary.PerUserProject.ShouldHaveSingleItem().ProjectTitle.ShouldBe("Website");
    }

    [Fact]
    public async Task Reports_Should_Keep_The_Name_Of_A_Deleted_Account_On_Its_Past_Time()
    {
        await CreateUserAsync(OtherUserId, "bob", "Bob", "Builder");
        var projectId = await SeedMyProjectAsync("Website");
        await InsertAsync(Block(projectId, Day.AddHours(9), 30, OtherUserId));
        await DeleteUserAsync(OtherUserId);

        var range = Range(Day, Day.AddDays(1));

        var bob = (await Reporting.GetSummaryAsync(range)).PerUser.ShouldHaveSingleItem();
        bob.UserName.ShouldBe("bob");
        bob.Name.ShouldBe("Bob Builder");
        (await Reporting.GetUserDailySeriesAsync(range)).ShouldHaveSingleItem().UserName.ShouldBe("bob");
        (await Reporting.GetDevicesAsync(range)).ShouldHaveSingleItem().UserName.ShouldBe("bob");
        (await Reporting.GetPayrollAsync(range)).Rows.ShouldHaveSingleItem().UserName.ShouldBe("bob");
    }

    [Fact]
    public async Task Summary_Should_Break_Time_Down_Per_Member_And_Project_With_Pay()
    {
        await CreateUserAsync(CurrentUserId, "ada");
        await CreateUserAsync(OtherUserId, "bob");
        var website = await SeedProjectAsync("Website");
        var app = await SeedProjectAsync("App");
        await AddRateAsync(website, CurrentUserId, 30m, LongAgo);
        await AddRateAsync(app, CurrentUserId, 45m, LongAgo);
        await AddRateAsync(website, OtherUserId, 20m, LongAgo);

        await InsertAsync(
            Block(website, Day.AddHours(9), 30, productivity: 100, configure: b => b.IdleSeconds = 60),
            Block(website, Day.AddHours(10), 30, productivity: 50),
            Block(app, Day.AddHours(11), 20, productivity: null),
            Block(website, Day.AddHours(9), 90, OtherUserId, productivity: 70));

        var rows = (await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)))).PerUserProject;

        // Ordered by member, then project.
        rows.Select(r => (r.UserName, r.ProjectTitle)).ShouldBe(new[] { ("ada", "App"), ("ada", "Website"), ("bob", "Website") });

        var adaWebsite = rows[1];
        adaWebsite.UserId.ShouldBe(CurrentUserId);
        adaWebsite.ProjectId.ShouldBe(website);
        adaWebsite.ActivityCount.ShouldBe(2);
        adaWebsite.TrackedMinutes.ShouldBe(60);
        adaWebsite.IdleMinutes.ShouldBe(1);
        adaWebsite.AverageProductivity.ShouldBe(75);
        adaWebsite.Amount.ShouldBe(30m);
        adaWebsite.HourlyRate.ShouldBe(30m);

        var adaApp = rows[0];
        adaApp.TrackedMinutes.ShouldBe(20);
        adaApp.AverageProductivity.ShouldBeNull();
        adaApp.Amount.ShouldBe(15m);

        rows[2].Amount.ShouldBe(30m); // 90 min @ 20
    }

    [Fact]
    public async Task Summary_Should_Filter_To_One_Member_For_Callers_Who_Can_View_All()
    {
        var projectId = await SeedMyProjectAsync();
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);
        await InsertAsync(Block(projectId, Day.AddHours(9), 20, OtherUserId, productivity: 50));

        var everyone = await Reporting.GetSummaryAsync(Range(Day, Day.AddDays(1)));
        everyone.PerUser.Count.ShouldBe(2);

        var input = Range(Day, Day.AddDays(1));
        input.UserId = OtherUserId;
        var onlyOther = await Reporting.GetSummaryAsync(input);
        onlyOther.TotalActivities.ShouldBe(1);
        onlyOther.TotalTrackedMinutes.ShouldBe(20);
        onlyOther.PerUser.ShouldHaveSingleItem().UserId.ShouldBe(OtherUserId);
    }

    // ---- Payroll ----

    [Fact]
    public async Task Payroll_Should_Price_Each_Block_At_The_Rate_In_Effect_At_Its_Start()
    {
        await CreateUserAsync(CurrentUserId, "ada");
        var projectId = await SeedMyProjectAsync("Website");
        var raise = Day.AddHours(12);
        await AddRateAsync(projectId, CurrentUserId, 40m, LongAgo);
        await AddRateAsync(projectId, CurrentUserId, 50m, raise);

        await InsertAsync(
            Block(projectId, Day.AddHours(9), 60),                  // before the raise: 40/h
            Block(projectId, raise.AddMinutes(-30), 60),            // straddles it, priced at its start: 40/h
            Block(projectId, raise.AddHours(1), 30),                // after: 50/h
            Block(projectId, Day.AddDays(1).AddHours(9), 60));      // outside the range

        var payroll = await Reporting.GetPayrollAsync(Range(Day, Day.AddDays(1)));

        var row = payroll.Rows.ShouldHaveSingleItem();
        row.UserName.ShouldBe("ada");
        row.ProjectTitle.ShouldBe("Website");
        row.TrackedMinutes.ShouldBe(150);
        row.Amount.ShouldBe(105m);                // 60×40/60 + 60×40/60 + 30×50/60 = 40 + 40 + 25
        row.HourlyRate.ShouldBe(42m);             // 105 / 2.5 h
        payroll.TotalMinutes.ShouldBe(150);
        payroll.TotalAmount.ShouldBe(105m);
    }

    [Fact]
    public async Task Payroll_Should_Keep_Pricing_Past_Time_After_A_Member_Is_Removed()
    {
        // Rate history outlives the membership: the departed member's tracked time keeps its price.
        var projectId = await SeedProjectAsync("Website");
        await AddRateAsync(projectId, OtherUserId, 60m, LongAgo);
        await InsertAsync(Block(projectId, Day.AddHours(9), 45, OtherUserId));

        var payroll = await Reporting.GetPayrollAsync(Range(Day, Day.AddDays(1)));

        payroll.Rows.ShouldHaveSingleItem().Amount.ShouldBe(45m);
    }

    [Fact]
    public async Task Payroll_Should_Fall_Back_To_The_Membership_Rate_Only_When_There_Is_No_Rate_History()
    {
        var historic = await SeedProjectAsync("Historic");
        await AddMemberAsync(historic, CurrentUserId, hourlyRate: 90m); // current rate; history below wins
        await AddRateAsync(historic, CurrentUserId, 30m, LongAgo);
        var noHistory = await SeedProjectAsync("No history");
        await AddMemberAsync(noHistory, CurrentUserId, hourlyRate: 24m);
        var unknown = await SeedProjectAsync("Unknown");                 // neither history nor membership

        await InsertAsync(
            Block(historic, Day.AddHours(9), 60),
            Block(noHistory, Day.AddHours(10), 30),
            Block(unknown, Day.AddHours(11), 30));

        var payroll = await Reporting.GetPayrollAsync(Range(Day, Day.AddDays(1)));

        payroll.Rows.Single(r => r.ProjectId == historic).Amount.ShouldBe(30m);
        payroll.Rows.Single(r => r.ProjectId == noHistory).Amount.ShouldBe(12m);
        payroll.Rows.Single(r => r.ProjectId == unknown).Amount.ShouldBe(0m);
        payroll.TotalAmount.ShouldBe(42m);
    }

    [Fact]
    public async Task Payroll_Total_Should_Be_The_Sum_Of_The_Rounded_Rows()
    {
        var projectA = await SeedProjectAsync("A");
        var projectB = await SeedProjectAsync("B");
        await AddRateAsync(projectA, CurrentUserId, 10m, LongAgo);
        await AddRateAsync(projectB, CurrentUserId, 10m, LongAgo);

        // 1 minute at 10/h = 0.1666… each; rounded per row to 0.17, so the total is 0.34 (not 0.33).
        await InsertAsync(Block(projectA, Day.AddHours(9), 1), Block(projectB, Day.AddHours(10), 1));

        var payroll = await Reporting.GetPayrollAsync(Range(Day, Day.AddDays(1)));

        payroll.Rows.ShouldAllBe(r => r.Amount == 0.17m);
        payroll.TotalAmount.ShouldBe(0.34m);
    }

    // ---- Daily series ----

    [Fact]
    public async Task DailySeries_Should_Cover_Every_Day_With_Zeros_For_Idle_Days()
    {
        var projectId = await SeedMyProjectAsync();

        await TrackAsync(projectId, Day.AddHours(9), 30, 80);            // day 0
        await InsertAsync(Block(projectId, Day.AddDays(2).AddHours(14), 45, productivity: 60, configure: b => b.IdleSeconds = 300));

        var series = await Reporting.GetDailySeriesAsync(Range(Day, Day.AddDays(3)));

        series.Count.ShouldBe(3);
        series[0].Date.ShouldBe(Day);
        series[0].Date.Kind.ShouldBe(DateTimeKind.Unspecified); // a local date, never an instant
        series[0].TrackedMinutes.ShouldBe(30);
        series[0].AverageProductivity.ShouldBe(80);
        series[0].ActivityCount.ShouldBe(1);

        series[1].TrackedMinutes.ShouldBe(0); // idle day still on the axis
        series[1].ActivityCount.ShouldBe(0);
        series[1].AverageProductivity.ShouldBeNull();

        series[2].TrackedMinutes.ShouldBe(45);
        series[2].IdleMinutes.ShouldBe(5);
        series[2].AverageProductivity.ShouldBe(60);
        series[2].MouseClicks.ShouldBe(10);
        series[2].KeyboardHits.ShouldBe(20);
    }

    [Fact]
    public async Task Day_Based_Reports_Should_Serialize_Local_Dates_Without_A_Zone_Suffix()
    {
        var json = GetRequiredService<IJsonSerializer>();
        var day = new DateTime(2026, 7, 14);

        json.Serialize(new DailyReportRowDto { Date = day }).ShouldContain("\"date\":\"2026-07-14T00:00:00\"");

        var userDay = json.Serialize(new UserDailyReportRowDto
        {
            Date = day,
            FirstStartedAt = new DateTime(2026, 7, 14, 3, 0, 0, DateTimeKind.Utc)
        });
        userDay.ShouldContain("\"date\":\"2026-07-14T00:00:00\"");
        userDay.ShouldContain("\"firstStartedAt\":\"2026-07-14T03:00:00Z\"");

        var attendance = json.Serialize(new AttendanceReportDto
        {
            Days = new List<DateTime> { day },
            Rows = new List<AttendanceRowDto> { new() { PresentDays = new List<DateTime> { day } } }
        });
        attendance.ShouldContain("\"days\":[\"2026-07-14T00:00:00\"]");
        attendance.ShouldContain("\"presentDays\":[\"2026-07-14T00:00:00\"]");
    }

    // ---- Per-member days ----

    [Fact]
    public async Task UserDailySeries_Should_Return_Minutes_Per_Member_Per_Day_With_Data_Only()
    {
        await CreateUserAsync(OtherUserId, "bob");

        var projectId = await SeedMyProjectAsync();
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);
        await InsertAsync(Block(projectId, Day.AddHours(10), 15, configure: b => b.IdleSeconds = 120));
        await InsertAsync(Block(projectId, Day.AddHours(9), 20, OtherUserId));
        await InsertAsync(Block(projectId, Day.AddDays(2).AddHours(9), 40, OtherUserId));

        var rows = await Reporting.GetUserDailySeriesAsync(Range(Day, Day.AddDays(3)));

        rows.Count.ShouldBe(3); // day 1 is idle for both members -> no rows

        var mineDay0 = rows.Single(r => r.UserId == CurrentUserId);
        mineDay0.Date.ShouldBe(Day);
        mineDay0.Date.Kind.ShouldBe(DateTimeKind.Unspecified);
        mineDay0.TrackedMinutes.ShouldBe(45);
        mineDay0.ActivityCount.ShouldBe(2);
        mineDay0.IdleMinutes.ShouldBe(2);
        mineDay0.FirstStartedAt.ShouldBe(Day.AddHours(9));
        mineDay0.FirstStartedAt.Kind.ShouldBe(DateTimeKind.Utc);
        mineDay0.LastEndedAt.ShouldBe(Day.AddHours(10).AddMinutes(15));
        mineDay0.LastEndedAt.Kind.ShouldBe(DateTimeKind.Utc);

        var bobRows = rows.Where(r => r.UserId == OtherUserId).OrderBy(r => r.Date).ToList();
        bobRows.Count.ShouldBe(2);
        bobRows.ShouldAllBe(r => r.UserName == "bob");
        bobRows[0].TrackedMinutes.ShouldBe(20);
        bobRows[1].Date.ShouldBe(Day.AddDays(2));
        bobRows[1].TrackedMinutes.ShouldBe(40);

        // Ordered by day first.
        rows.Select(r => r.Date).ShouldBe(rows.Select(r => r.Date).OrderBy(d => d));

        var input = Range(Day, Day.AddDays(3));
        input.UserId = OtherUserId;
        var onlyBob = await Reporting.GetUserDailySeriesAsync(input);
        onlyBob.ShouldAllBe(r => r.UserId == OtherUserId);
        onlyBob.Count.ShouldBe(2);
    }

    // ---- Range validation ----

    [Fact]
    public async Task Reports_Should_Reject_A_Range_That_Ends_Before_It_Starts()
    {
        var exception = await Should.ThrowAsync<Volo.Abp.BusinessException>(() =>
            Reporting.GetSummaryAsync(Range(Day, Day)));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);

        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetDailySeriesAsync(Range(Day, Day.AddDays(-1))));
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetPayrollAsync(Range(Day, Day)));
    }

    [Fact]
    public async Task Reports_Should_Cap_The_Range_At_366_Days()
    {
        var tooLong = Range(Day, Day.AddDays(GetReportSummaryInput.MaxRangeDays + 1));

        var exception = await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetAttendanceAsync(tooLong));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetAppUsageAsync(tooLong));
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetUserDailySeriesAsync(tooLong));
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetPayrollAsync(tooLong));
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetHourlyAsync(tooLong));
        await Should.ThrowAsync<Volo.Abp.BusinessException>(() => Reporting.GetDevicesAsync(tooLong));

        var fullYear = await Reporting.GetDailySeriesAsync(Range(Day, Day.AddDays(GetReportSummaryInput.MaxRangeDays)));
        fullYear.Count.ShouldBe(GetReportSummaryInput.MaxRangeDays);
    }

    // ---- App usage ----

    [Fact]
    public async Task AppUsage_Should_Credit_Focused_Time_And_Count_Reach()
    {
        var projectId = await SeedMyProjectAsync();
        // Block 1: focused on code.exe (30m), with chrome.exe also running in the background.
        await TrackWithAppsAsync(projectId, Day.AddHours(9), 30, new[] { "code.exe" }, new[] { "code.exe", "chrome.exe" });
        // Block 2: focused on chrome.exe (20m).
        await TrackWithAppsAsync(projectId, Day.AddHours(10), 20, new[] { "chrome.exe" }, new[] { "chrome.exe" });

        var apps = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        apps.Count.ShouldBe(2);
        apps[0].AppName.ShouldBe("code.exe"); // most focused minutes first
        apps[0].DisplayName.ShouldBe("Visual Studio Code");
        apps[0].TrackedMinutes.ShouldBe(30);
        apps[0].ActivityCount.ShouldBe(1);
        apps[0].UserCount.ShouldBe(1);

        var chrome = apps.Single(a => a.AppName == "chrome.exe");
        chrome.DisplayName.ShouldBe("Google Chrome");
        chrome.TrackedMinutes.ShouldBe(20);       // focused only in block 2
        chrome.ActivityCount.ShouldBe(2);         // running in block 1, focused in block 2
    }

    [Fact]
    public async Task AppUsage_Should_Credit_Per_Window_Seconds_When_Reported_And_Fall_Back_For_Legacy_Blocks()
    {
        var projectId = await SeedMyProjectAsync();
        // New agent, 30-minute block: code.exe 15m + 5m across two titles, chrome.exe 10m.
        await TrackWithWindowsAsync(projectId, Day.AddHours(9), 30,
            ("code.exe", 900), ("chrome.exe", 600), ("code.exe", 300), ("notepad.exe", 0)); // notepad: seen, no foreground time
        // Legacy agent, 20-minute block: only the focused window, no seconds -> credited the whole block.
        await TrackWithAppsAsync(projectId, Day.AddHours(10), 20, new[] { "chrome.exe" }, new[] { "chrome.exe", "slack.exe" });

        var apps = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        apps.Count.ShouldBe(4);
        apps[0].AppName.ShouldBe("chrome.exe"); // most credited minutes first
        apps[1].AppName.ShouldBe("code.exe");

        var chrome = apps.Single(a => a.AppName == "chrome.exe");
        chrome.TrackedMinutes.ShouldBe(30); // 10 (seconds) + 20 (legacy block)
        chrome.ActivityCount.ShouldBe(2);
        chrome.UserCount.ShouldBe(1);

        var code = apps.Single(a => a.AppName == "code.exe");
        code.TrackedMinutes.ShouldBe(20); // 900 s + 300 s, not the whole 30-minute block
        code.ActivityCount.ShouldBe(1);   // two titles in one block still count once

        apps.Single(a => a.AppName == "notepad.exe").TrackedMinutes.ShouldBe(0);
        apps.Single(a => a.AppName == "notepad.exe").ActivityCount.ShouldBe(1);
        apps.Single(a => a.AppName == "slack.exe").TrackedMinutes.ShouldBe(0); // running only
    }

    [Fact]
    public async Task AppUsage_Should_Never_Credit_A_Block_More_Minutes_Than_It_Lasted()
    {
        var projectId = await SeedMyProjectAsync();
        // A misbehaving agent reports 10 + 10 minutes of foreground time inside a 10-minute block.
        await TrackWithWindowsAsync(projectId, Day.AddHours(9), 10, ("code.exe", 600), ("chrome.exe", 600));

        var apps = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        apps.Sum(a => a.TrackedMinutes).ShouldBe(10);
        apps.ShouldAllBe(a => a.TrackedMinutes == 5);
    }

    [Fact]
    public async Task AppUsage_Should_Group_Every_Spelling_Of_An_App_Under_Its_Canonical_Name()
    {
        var projectId = await SeedMyProjectAsync();
        await AddMemberAsync(projectId, OtherUserId);

        // Windows reports the executable, macOS the app's name: one app, one row.
        await TrackWithAppsAsync(projectId, Day.AddHours(9), 40, new[] { "chrome.exe" }, new[] { "chrome.exe", "Code.exe" });
        await TrackWithAppsAsync(projectId, Day.AddHours(9), 15, new[] { "Google Chrome" }, new[] { "Google Chrome", "Code" }, OtherUserId);
        await TrackWithAppsAsync(projectId, Day.AddHours(10), 10, new[] { "CHROME.EXE" }, Array.Empty<string>());

        var apps = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        var chrome = apps.Single(a => a.DisplayName == "Google Chrome");
        chrome.AppName.ShouldBe("chrome.exe"); // the most credited spelling
        chrome.TrackedMinutes.ShouldBe(65);
        chrome.ActivityCount.ShouldBe(3);
        chrome.UserCount.ShouldBe(2);

        var code = apps.Single(a => a.DisplayName == "Visual Studio Code");
        code.TrackedMinutes.ShouldBe(0);
        code.ActivityCount.ShouldBe(2);
        code.UserCount.ShouldBe(2);

        apps.Count.ShouldBe(2);
    }

    [Fact]
    public async Task AppUsage_Should_Report_Idle_Minutes_Per_App()
    {
        var projectId = await SeedMyProjectAsync();

        // 10-minute block, 6 minutes in Code then 4 in Chrome. The agent measured 4 idle minutes; the timeline places
        // 3 of them (minutes 7-9, in Chrome), and the 4th is shared by the apps in proportion to their time.
        var apps = new[] { "code.exe", "code.exe", "code.exe", "code.exe", "code.exe", "code.exe", "chrome.exe", "chrome.exe", "chrome.exe", "chrome.exe" };
        await InsertAsync(Block(projectId, Day.AddHours(9), 10, configure: b =>
        {
            b.ActiveWindowsJson = StoredWindows(("code.exe", 360), ("chrome.exe", 240));
            b.TimelineJson = StoredTimeline(apps, 7, 8, 9);
            b.IdleSeconds = 240;
        }));

        // Legacy block: its single app is given the whole measured idle time.
        await InsertAsync(Block(projectId, Day.AddHours(10), 20, configure: b =>
        {
            b.ActiveWindowsJson = StoredWindows(("slack.exe", null));
            b.IdleSeconds = 300;
        }));

        // Unmeasured idle time: nothing is idle.
        await TrackWithAppsAsync(projectId, Day.AddHours(11), 5, new[] { "notepad.exe" }, Array.Empty<string>());

        var rows = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        rows.Single(a => a.DisplayName == "Google Chrome").IdleMinutes.ShouldBe(3.4); // 3 placed + 1 × 4/10
        rows.Single(a => a.DisplayName == "Visual Studio Code").IdleMinutes.ShouldBe(0.6); // 1 × 6/10
        rows.Single(a => a.DisplayName == "Slack").IdleMinutes.ShouldBe(5);
        rows.Single(a => a.DisplayName == "Notepad").IdleMinutes.ShouldBe(0);
        rows.ShouldAllBe(a => a.IdleMinutes <= a.TrackedMinutes);
    }

    [Fact]
    public async Task AppUsage_Should_Never_Give_An_App_More_Idle_Time_Than_It_Was_Credited()
    {
        var projectId = await SeedMyProjectAsync();

        // The timeline says every minute was idle in Code, but Code was only credited 2 of the 10 minutes.
        await InsertAsync(Block(projectId, Day.AddHours(9), 10, configure: b =>
        {
            b.ActiveWindowsJson = StoredWindows(("code.exe", 120), ("chrome.exe", 480));
            b.TimelineJson = StoredTimeline(Enumerable.Repeat("code.exe", 10).ToList(), 0, 1, 2, 3, 4, 5, 6, 7, 8, 9);
            b.IdleSeconds = 600;
        }));

        var rows = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        rows.Single(a => a.DisplayName == "Visual Studio Code").IdleMinutes.ShouldBe(2);
        rows.Single(a => a.DisplayName == "Google Chrome").IdleMinutes.ShouldBe(0);
    }

    [Fact]
    public async Task AppUsage_Should_Ignore_Window_Data_It_Cannot_Read()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Day.AddHours(9), 10, configure: b => b.ActiveWindowsJson = "not json"),
            Block(projectId, Day.AddHours(10), 10, configure: b =>
            {
                b.ActiveWindowsJson = "[null, {\"AppName\": \"  \"}, {\"AppName\": \".exe\"}, {\"AppName\": \"slack.exe\"}]";
                b.TimelineJson = "{\"not\": \"a list\"}";
                b.IdleSeconds = 60;
            }));

        var rows = await Reporting.GetAppUsageAsync(Range(Day, Day.AddDays(1)));

        var slack = rows.ShouldHaveSingleItem();
        slack.DisplayName.ShouldBe("Slack");
        slack.TrackedMinutes.ShouldBe(10);
        slack.IdleMinutes.ShouldBe(1);
    }

    // ---- Hourly ----

    [Fact]
    public async Task Hourly_Should_Split_Blocks_Across_Hour_Boundaries()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Day.AddHours(9).AddMinutes(40), 40, productivity: 50),   // Monday 09:40-10:20
            Block(projectId, Day.AddHours(10).AddMinutes(30), 20, productivity: null)); // unmeasured: tracked only

        var rows = await Reporting.GetHourlyAsync(Range(Day, Day.AddDays(1)));

        rows.Count.ShouldBe(7 * 24);
        rows.Select(r => (r.DayOfWeek, r.Hour)).ShouldBe(
            Enumerable.Range(0, 7).SelectMany(d => Enumerable.Range(0, 24).Select(h => (d, h))));

        var nine = rows.Single(r => r.DayOfWeek == (int)DayOfWeek.Monday && r.Hour == 9);
        nine.TrackedMinutes.ShouldBe(20);
        nine.ActiveMinutes.ShouldBe(10);
        var ten = rows.Single(r => r.DayOfWeek == (int)DayOfWeek.Monday && r.Hour == 10);
        ten.TrackedMinutes.ShouldBe(40);
        ten.ActiveMinutes.ShouldBe(10);

        rows.Sum(r => r.TrackedMinutes).ShouldBe(60);
    }

    // ---- Devices ----

    [Fact]
    public async Task Devices_Should_Group_By_Member_And_Device_With_The_Latest_Details()
    {
        await CreateUserAsync(CurrentUserId, "ada");
        await CreateUserAsync(OtherUserId, "bob");
        var projectId = await SeedMyProjectAsync();
        var laptop = Guid.NewGuid();
        var desktop = Guid.NewGuid();

        void Device(Activity block, Guid id, string name, string platform, string os, string version)
        {
            block.DeviceId = id;
            block.DeviceName = name;
            block.Platform = platform;
            block.OsVersion = os;
            block.AgentVersion = version;
        }

        await InsertAsync(
            Block(projectId, Day.AddHours(9), 30, configure: b => Device(b, laptop, "ADA-LAPTOP", "windows", "Windows 11", "0.1.0")),
            Block(projectId, Day.AddHours(14), 20, configure: b => Device(b, laptop, "ADA-LAPTOP", "windows", "Windows 11", "0.2.0")),
            Block(projectId, Day.AddHours(11), 15, configure: b => Device(b, desktop, "Ada's iMac", "macos", "macOS 14.5", "0.2.0")),
            Block(projectId, Day.AddHours(12), 10),                    // an older agent: no device details
            Block(projectId, Day.AddHours(9), 45, OtherUserId));

        var devices = await Reporting.GetDevicesAsync(Range(Day, Day.AddDays(1)));

        devices.Count.ShouldBe(4);
        devices.Select(d => d.UserName).ShouldBe(new[] { "ada", "ada", "ada", "bob" });

        // Ada's devices, most recently used first.
        devices[0].DeviceId.ShouldBe(laptop);
        devices[0].AgentVersion.ShouldBe("0.2.0"); // the latest block's version
        devices[0].Platform.ShouldBe("windows");
        devices[0].DeviceName.ShouldBe("ADA-LAPTOP");
        devices[0].TrackedMinutes.ShouldBe(50);
        devices[0].LastActivityAt.ShouldBe(Day.AddHours(14).AddMinutes(20));
        devices[0].LastActivityAt.Kind.ShouldBe(DateTimeKind.Utc);

        devices[1].DeviceId.ShouldBeNull();
        devices[1].TrackedMinutes.ShouldBe(10);
        devices[1].Platform.ShouldBeNull();

        devices[2].DeviceId.ShouldBe(desktop);
        devices[2].OsVersion.ShouldBe("macOS 14.5");

        devices[3].UserId.ShouldBe(OtherUserId);
        devices[3].TrackedMinutes.ShouldBe(45);
    }
}
