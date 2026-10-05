using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Activities;

/// <summary>
/// A caller who sees everyone's activity (Activities.ViewAll) but does not manage the team (no Team.Manage): other
/// members' notes are visible, yet only their owners or team managers may change them.
/// </summary>
public abstract class ActivityNoteViewerTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000012");

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task UpdateNote_Should_Refuse_Another_Members_Block()
    {
        var start = DateTime.UtcNow.AddDays(-1);
        var block = new Activity(Guid.NewGuid(), null, OtherUserId, Guid.NewGuid(), Guid.NewGuid(), start, start.AddMinutes(10))
        {
            Description = "Initial note"
        };
        var repository = GetRequiredService<IRepository<Activity, Guid>>();
        await WithUnitOfWorkAsync(() => repository.InsertAsync(block));

        await Should.ThrowAsync<AbpAuthorizationException>(() => GetRequiredService<IActivityAppService>()
            .UpdateNoteAsync(block.Id, new UpdateActivityNoteDto { Description = "Edited by a viewer" }));

        (await WithUnitOfWorkAsync(() => repository.GetAsync(block.Id))).Description.ShouldBe("Initial note");
    }
}
