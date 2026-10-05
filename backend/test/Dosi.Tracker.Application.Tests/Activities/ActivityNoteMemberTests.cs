using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Activities;

/// <summary>
/// The task note of a block (<c>PUT /api/app/activity/{id}/note</c>) for a regular member: no Activities.ViewAll and
/// no Team.Manage. The owner may change it within 30 days of the block's start; other members' blocks stay hidden.
/// </summary>
public abstract class ActivityNoteMemberTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    private static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000011");

    private readonly IActivityAppService _activityAppService;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected ActivityNoteMemberTests()
    {
        _activityAppService = GetRequiredService<IActivityAppService>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Team.Manage);
    }

    private async Task<Activity> SeedBlockAsync(Guid userId, DateTime start)
    {
        var activity = new Activity(
            _guidGenerator.Create(), null, userId, _guidGenerator.Create(), Guid.NewGuid(), start, start.AddMinutes(10))
        {
            Description = "Initial note"
        };
        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(activity));
        return activity;
    }

    [Fact]
    public async Task UpdateNote_Should_Let_The_Owner_Change_A_Block_Of_The_Last_30_Days()
    {
        var block = await SeedBlockAsync(CurrentUserId, DateTime.UtcNow.AddDays(-29));

        var updated = await _activityAppService.UpdateNoteAsync(block.Id, new UpdateActivityNoteDto { Description = "Sprint review" });

        updated.Description.ShouldBe("Sprint review");
    }

    [Fact]
    public async Task UpdateNote_Should_Refuse_The_Owner_After_30_Days()
    {
        var block = await SeedBlockAsync(CurrentUserId, DateTime.UtcNow.AddDays(-31));

        await Should.ThrowAsync<AbpAuthorizationException>(() =>
            _activityAppService.UpdateNoteAsync(block.Id, new UpdateActivityNoteDto { Description = "Too late" }));

        (await WithUnitOfWorkAsync(() => _activityRepository.GetAsync(block.Id))).Description.ShouldBe("Initial note");
    }

    [Fact]
    public async Task UpdateNote_Should_Hide_Other_Members_Blocks()
    {
        var block = await SeedBlockAsync(OtherUserId, DateTime.UtcNow.AddDays(-1));

        await Should.ThrowAsync<EntityNotFoundException>(() =>
            _activityAppService.UpdateNoteAsync(block.Id, new UpdateActivityNoteDto { Description = "Not mine" }));

        (await WithUnitOfWorkAsync(() => _activityRepository.GetAsync(block.Id))).Description.ShouldBe("Initial note");
    }
}
