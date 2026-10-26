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
/// <c>GET /api/app/reporting/data-quality</c> as a caller who sees the whole team (the test authorization grants every
/// permission): per member, the blocks, tracked and unmeasured minutes, largest clock skew, blocks with
/// software-generated input and the latest end.
/// </summary>
public abstract class ReportingDataQualityTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid CarolId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000d3");

    [Fact]
    public async Task Should_Fold_Each_Members_Blocks_In_The_Range()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(OtherUserId, "bob");
        await CreateUserAsync(CarolId, "carol"); // a member with no blocks gets no row
        var project = await SeedMyProjectAsync("Website");
        await AddMemberAsync(project, OtherUserId);
        await AddMemberAsync(project, CarolId);

        await InsertAsync(
            Block(project, Day.AddHours(9), 30, configure: b => b.ClockSkewSeconds = -400),
            Block(project, Day.AddHours(10), 20, productivity: null, configure: b =>
            {
                b.ClockSkewSeconds = 120;
                b.InjectedInputEvents = 5;
            }),
            Block(project, Day.AddHours(11), 10.5, productivity: 50, configure: b => b.InjectedInputEvents = 0),
            Block(project, Day.AddHours(8), 15, OtherUserId, productivity: null),
            // Outside the range (by start): not counted.
            Block(project, Day.AddDays(-1).AddHours(23), 90, configure: b => b.ClockSkewSeconds = 9000),
            Block(project, Day.AddDays(1), 30, OtherUserId, configure: b => b.InjectedInputEvents = 3));

        var rows = await Reporting.GetDataQualityAsync(Range(Day, Day.AddDays(1)));

        rows.Select(r => r.UserName).ShouldBe(new[] { "bob", "me" });

        var mine = rows.Single(r => r.UserId == CurrentUserId);
        mine.Blocks.ShouldBe(3);
        mine.TrackedMinutes.ShouldBe(60.5);
        mine.UnmeasuredMinutes.ShouldBe(20);
        mine.MaxAbsClockSkewSeconds.ShouldBe(400);
        mine.BlocksWithInjectedInput.ShouldBe(1);
        mine.LastActivityAt.ShouldBe(Day.AddHours(11).AddMinutes(10.5));
        mine.LastActivityAt!.Value.Kind.ShouldBe(DateTimeKind.Utc);

        var bob = rows.Single(r => r.UserId == OtherUserId);
        bob.Blocks.ShouldBe(1);
        bob.TrackedMinutes.ShouldBe(15);
        bob.UnmeasuredMinutes.ShouldBe(15);
        bob.MaxAbsClockSkewSeconds.ShouldBeNull();
        bob.BlocksWithInjectedInput.ShouldBe(0);
        bob.LastActivityAt.ShouldBe(Day.AddHours(8).AddMinutes(15));
    }

    [Fact]
    public async Task Should_Narrow_To_The_Requested_Member_And_Project()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(OtherUserId, "bob");
        var website = await SeedMyProjectAsync("Website");
        var backOffice = await SeedMyProjectAsync("Back office");
        await AddMemberAsync(website, OtherUserId);

        await InsertAsync(
            Block(website, Day.AddHours(9), 30),
            Block(backOffice, Day.AddHours(10), 20),
            Block(website, Day.AddHours(11), 15, OtherUserId));

        var input = Range(Day, Day.AddDays(1));
        input.UserId = OtherUserId;
        (await Reporting.GetDataQualityAsync(input)).ShouldHaveSingleItem().TrackedMinutes.ShouldBe(15);

        input.UserId = null;
        input.ProjectId = backOffice;
        var row = (await Reporting.GetDataQualityAsync(input)).ShouldHaveSingleItem();
        row.UserId.ShouldBe(CurrentUserId);
        row.TrackedMinutes.ShouldBe(20);
    }

    [Fact]
    public async Task Should_Return_No_Rows_For_An_Empty_Range()
    {
        (await Reporting.GetDataQualityAsync(Range(Day, Day.AddDays(1)))).ShouldBeEmpty();
    }

    [Fact]
    public async Task Should_Refuse_An_Invalid_Range()
    {
        var error = await Should.ThrowAsync<Volo.Abp.BusinessException>(() =>
            Reporting.GetDataQualityAsync(Range(Day, Day)));

        error.Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
    }
}

/// <summary>
/// The data-quality report as a caller without Tracker.Activities.ViewAll and Tracker.Team.Manage: a worker gets only
/// their own row, whoever they ask for; a client of a project also gets that project's members.
/// </summary>
public abstract class ReportingDataQualityScopeTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid CarolId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000d4");

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task A_Worker_Should_Only_Get_Their_Own_Row_Even_When_Asking_For_Someone_Else()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(OtherUserId, "bob");
        var project = await SeedMyProjectAsync("Website");
        await AddMemberAsync(project, OtherUserId);
        await InsertAsync(
            Block(project, Day.AddHours(9), 30, configure: b => b.ClockSkewSeconds = 30),
            Block(project, Day.AddHours(10), 20, OtherUserId, configure: b => b.ClockSkewSeconds = 900));

        var input = Range(Day, Day.AddDays(1));
        input.UserId = OtherUserId; // cannot widen the scope

        var row = (await Reporting.GetDataQualityAsync(input)).ShouldHaveSingleItem();
        row.UserId.ShouldBe(CurrentUserId);
        row.TrackedMinutes.ShouldBe(30);
        row.MaxAbsClockSkewSeconds.ShouldBe(30);
    }

    [Fact]
    public async Task A_Client_Should_Get_The_Members_Of_Their_Client_Project()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(OtherUserId, "bob");
        await CreateUserAsync(CarolId, "carol");

        var clientProject = await SeedProjectAsync("Client project");
        await AddMemberAsync(clientProject, CurrentUserId, TrackerRoles.ClientProjectRole);
        await AddMemberAsync(clientProject, OtherUserId);
        var foreign = await SeedProjectAsync("Foreign");
        await AddMemberAsync(foreign, CarolId);

        await InsertAsync(
            Block(clientProject, Day.AddHours(10), 20, OtherUserId, productivity: null),
            Block(foreign, Day.AddHours(11), 25, CarolId));

        var row = (await Reporting.GetDataQualityAsync(Range(Day, Day.AddDays(1)))).ShouldHaveSingleItem();
        row.UserName.ShouldBe("bob");
        row.UnmeasuredMinutes.ShouldBe(20);
    }
}

/// <summary>A caller who sees everyone's activity but holds no Tracker.Reporting: only their own row is answered.</summary>
public abstract class ReportingDataQualityWithoutReportingTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Reporting.Default);
    }

    [Fact]
    public async Task Should_Refuse_The_Team_And_Answer_The_Caller()
    {
        var project = await SeedMyProjectAsync();
        await InsertAsync(Block(project, Day.AddHours(9), 30));

        await Should.ThrowAsync<AbpAuthorizationException>(() => Reporting.GetDataQualityAsync(Range(Day, Day.AddDays(1))));

        var own = Range(Day, Day.AddDays(1));
        own.UserId = CurrentUserId;
        (await Reporting.GetDataQualityAsync(own)).ShouldHaveSingleItem().Blocks.ShouldBe(1);
    }
}
