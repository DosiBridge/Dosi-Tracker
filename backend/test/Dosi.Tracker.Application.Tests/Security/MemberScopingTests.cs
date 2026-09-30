using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Teams;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// The caller here is a regular member: no Activities.ViewAll, Team.Manage or Projects.Edit
/// (every other permission is still allowed). Verifies the in-method data scoping that the
/// always-allow test authorization would otherwise hide.
/// </summary>
public abstract class MemberScopingTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    private static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000003");
    private static readonly DateTime Day = new(2026, 7, 20, 0, 0, 0, DateTimeKind.Utc);

    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected MemberScopingTests()
    {
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(
            TrackerPermissions.Activities.ViewAll,
            TrackerPermissions.Team.Manage,
            TrackerPermissions.Projects.Edit);
    }

    /// <summary>A project the current user belongs to and one they do not; one block each for both users.</summary>
    private async Task<(Guid Mine, Guid Foreign)> SeedAsync()
    {
        var mine = _guidGenerator.Create();
        var foreign = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            await _projectRepository.InsertAsync(new Project(mine, null, "Mine"));
            await _projectRepository.InsertAsync(new Project(foreign, null, "Foreign"));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, mine, CurrentUserId));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, mine, OtherUserId));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, foreign, OtherUserId));

            await _activityRepository.InsertAsync(new Activity(
                _guidGenerator.Create(), null, CurrentUserId, mine, Guid.NewGuid(), Day.AddHours(9), Day.AddHours(9).AddMinutes(10)));
            await _activityRepository.InsertAsync(new Activity(
                _guidGenerator.Create(), null, OtherUserId, mine, Guid.NewGuid(), Day.AddHours(9), Day.AddHours(9).AddMinutes(20)));
        });

        return (mine, foreign);
    }

    [Fact]
    public async Task Reports_Ignore_The_UserId_Filter_And_Only_Show_The_Callers_Own_Data()
    {
        await SeedAsync();
        var reporting = GetRequiredService<IReportingAppService>();

        var summary = await reporting.GetSummaryAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(1),
            UserId = OtherUserId // must not let a regular member peek at a colleague
        });
        summary.PerUser.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
        summary.TotalTrackedMinutes.ShouldBe(10);

        var perDay = await reporting.GetUserDailySeriesAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(1),
            UserId = OtherUserId
        });
        perDay.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task Team_Members_Lists_Only_The_Caller()
    {
        var (mine, _) = await SeedAsync();
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.CreateAsync(new IdentityUser(CurrentUserId, "me", "me@acme.test"))).Succeeded.ShouldBeTrue();
            (await userManager.CreateAsync(new IdentityUser(OtherUserId, "colleague", "colleague@acme.test"))).Succeeded.ShouldBeTrue();
        });

        var members = await GetRequiredService<ITeamAppService>().GetMembersAsync();

        var me = members.Items.ShouldHaveSingleItem();
        me.UserId.ShouldBe(CurrentUserId);
        me.UserName.ShouldBe("me");
        me.ProjectIds.ShouldBe(new[] { mine });
    }

    [Fact]
    public async Task Projects_Are_Limited_To_Those_The_Caller_Belongs_To()
    {
        var (mine, foreign) = await SeedAsync();
        var projects = GetRequiredService<IProjectAppService>();

        var list = await projects.GetListAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 100 });
        list.TotalCount.ShouldBe(1);
        list.Items.ShouldHaveSingleItem().Id.ShouldBe(mine);

        (await projects.GetAsync(mine)).Title.ShouldBe("Mine");
        await Should.ThrowAsync<EntityNotFoundException>(() => projects.GetAsync(foreign));
    }

    [Fact]
    public async Task Activity_List_Is_Limited_To_The_Callers_Own_Blocks()
    {
        await SeedAsync();

        var page = await GetRequiredService<IActivityAppService>().GetListAsync(new GetActivitiesInput
        {
            UserId = OtherUserId,
            MaxResultCount = 100
        });

        page.Items.ShouldAllBe(a => a.UserId == CurrentUserId);
        page.TotalCount.ShouldBe(1);
    }
}
