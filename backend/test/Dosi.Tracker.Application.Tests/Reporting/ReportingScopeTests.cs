using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The reports as a caller without Tracker.Activities.ViewAll and Tracker.Team.Manage: a worker sees only their
/// own data, and a member who is a client of projects also sees every member's time on those projects.
/// </summary>
public abstract class ReportingScopeTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid CarolId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000c2");

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task A_Worker_Should_Only_Ever_See_Their_Own_Data()
    {
        await CreateUserAsync(OtherUserId, "bob");
        var project = await SeedMyProjectAsync("Website");
        await AddMemberAsync(project, OtherUserId);
        await AddRateAsync(project, CurrentUserId, 30m, LongAgo);
        await InsertAsync(
            Block(project, Day.AddHours(9), 30, configure: b => b.ActiveWindowsJson = StoredWindows(("code.exe", null))),
            Block(project, Day.AddHours(10), 20, OtherUserId, configure: b => b.ActiveWindowsJson = StoredWindows(("slack.exe", null))));

        var input = Range(Day, Day.AddDays(1));
        input.UserId = OtherUserId; // cannot widen the scope

        var summary = await Reporting.GetSummaryAsync(input);
        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.PerUser.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);

        // Pay is Team.Manage data: the minutes are there, the money is not.
        var mine = summary.PerUserProject.ShouldHaveSingleItem();
        mine.TrackedMinutes.ShouldBe(30);
        mine.Amount.ShouldBe(0m);
        mine.HourlyRate.ShouldBe(0m);

        (await Reporting.GetDailySeriesAsync(input)).Sum(d => d.TrackedMinutes).ShouldBe(30);
        (await Reporting.GetUserDailySeriesAsync(input)).ShouldAllBe(r => r.UserId == CurrentUserId);
        (await Reporting.GetAppUsageAsync(input)).ShouldHaveSingleItem().DisplayName.ShouldBe("Visual Studio Code");
        (await Reporting.GetHourlyAsync(input)).Sum(h => h.TrackedMinutes).ShouldBe(30);
        (await Reporting.GetDevicesAsync(input)).ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
        (await Reporting.GetAttendanceAsync(input)).Rows.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task A_Client_Should_See_Everyone_On_Their_Client_Projects_And_Their_Own_Time_Elsewhere()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(OtherUserId, "bob");
        await CreateUserAsync(CarolId, "carol");

        var clientProject = await SeedProjectAsync("Client project");
        await AddMemberAsync(clientProject, CurrentUserId, TrackerRoles.ClientProjectRole);
        await AddMemberAsync(clientProject, OtherUserId);
        await AddMemberAsync(clientProject, CarolId, "client");
        var shared = await SeedMyProjectAsync("Shared");
        await AddMemberAsync(shared, OtherUserId);
        var foreign = await SeedProjectAsync("Foreign");
        await AddMemberAsync(foreign, OtherUserId);

        await InsertAsync(
            Block(shared, Day.AddHours(9), 30),                     // mine: visible
            Block(clientProject, Day.AddHours(10), 20, OtherUserId), // on my client project: visible
            Block(shared, Day.AddHours(11), 15, OtherUserId),        // a colleague on a project I only work on: hidden
            Block(foreign, Day.AddHours(12), 25, OtherUserId));      // elsewhere: hidden

        var range = Range(Day, Day.AddDays(1));
        var summary = await Reporting.GetSummaryAsync(range);

        summary.TotalTrackedMinutes.ShouldBe(50);
        summary.PerUser.Select(u => (u.UserName, u.TrackedMinutes)).ShouldBe(new[] { ("me", 30.0), ("bob", 20.0) });
        summary.PerProject.Select(p => (p.ProjectTitle, p.TrackedMinutes)).ShouldBe(new[] { ("Shared", 30.0), ("Client project", 20.0) });

        // Filters narrow the client view, never widen it.
        range.UserId = OtherUserId;
        (await Reporting.GetSummaryAsync(range)).TotalTrackedMinutes.ShouldBe(20);
        range.UserId = CurrentUserId;
        (await Reporting.GetSummaryAsync(range)).TotalTrackedMinutes.ShouldBe(30);
        range.UserId = null;
        range.ProjectId = foreign;
        (await Reporting.GetSummaryAsync(range)).TotalActivities.ShouldBe(0);
        range.ProjectId = clientProject;
        (await Reporting.GetSummaryAsync(range)).PerUser.ShouldHaveSingleItem().UserId.ShouldBe(OtherUserId);

        // The roster: the client project's members (clients excluded) and the caller as a member elsewhere.
        var attendance = await Reporting.GetAttendanceAsync(Range(Day, Day.AddDays(1)));
        attendance.Rows.Select(r => r.UserName).ShouldBe(new[] { "me", "bob" });
        attendance.Rows.Single(r => r.UserName == "bob").WorkedMinutes.ShouldBe(20);
    }

    [Fact]
    public async Task Payroll_Should_Require_The_Team_Manage_Permission()
    {
        var project = await SeedMyProjectAsync();
        await InsertAsync(Block(project, Day.AddHours(9), 30));

        await Should.ThrowAsync<AbpAuthorizationException>(() => Reporting.GetPayrollAsync(Range(Day, Day.AddDays(1))));
    }
}
