using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Activities;

/// <summary>
/// <see cref="ActivityNoteRules"/> directly, per kind of caller: who may change a block's note, and which blocks a note
/// suggestion may be made for (the edit rule, plus Tracker.Activities.ViewAll for another member's block).
/// </summary>
public abstract class ActivityNoteRulesTestBase<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    protected static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    protected static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000021");

    protected ActivityNoteRules Rules => GetRequiredService<ActivityNoteRules>();

    protected async Task<Activity> SeedBlockAsync(Guid userId, int daysAgo)
    {
        var start = DateTime.UtcNow.AddDays(-daysAgo);
        var activity = new Activity(
            Guid.NewGuid(), TrackerTestConsts.WorkspaceId, userId, Guid.NewGuid(), Guid.NewGuid(), start, start.AddMinutes(10))
        {
            Description = "Initial note"
        };
        await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Activity, Guid>>().InsertAsync(activity));
        return activity;
    }

    protected Task<Activity> LoadAsync(Guid id) => WithUnitOfWorkAsync(() => Rules.LoadForSuggestionAsync(id));
}

/// <summary>A team manager who sees everyone (the test authorization grants every permission).</summary>
public abstract class ActivityNoteRulesTests<TStartupModule> : ActivityNoteRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public void The_Edit_Window_Should_Be_Thirty_Days_And_Shared_With_The_Endpoint()
    {
        ActivityNoteRules.NoteEditWindow.ShouldBe(TimeSpan.FromDays(30));
        ActivityAppService.NoteEditWindow.ShouldBe(ActivityNoteRules.NoteEditWindow);
    }

    [Fact]
    public async Task Should_Allow_Any_Block_At_Any_Age()
    {
        var oldOwn = await SeedBlockAsync(CurrentUserId, daysAgo: 90);
        var others = await SeedBlockAsync(OtherUserId, daysAgo: 90);

        await Rules.EnsureCanEditNoteAsync(oldOwn);
        await Rules.EnsureCanEditNoteAsync(others);

        (await LoadAsync(oldOwn.Id)).Id.ShouldBe(oldOwn.Id);
        var loaded = await LoadAsync(others.Id);
        loaded.UserId.ShouldBe(OtherUserId);
        loaded.Description.ShouldBe("Initial note");
    }

    [Fact]
    public async Task Load_Should_Hide_A_Block_That_Does_Not_Exist()
    {
        await Should.ThrowAsync<EntityNotFoundException>(() => LoadAsync(Guid.NewGuid()));
    }
}

/// <summary>A regular member: no Tracker.Activities.ViewAll and no Tracker.Team.Manage.</summary>
public abstract class ActivityNoteRulesMemberTests<TStartupModule> : ActivityNoteRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task Should_Allow_Their_Own_Block_Within_Thirty_Days()
    {
        var block = await SeedBlockAsync(CurrentUserId, daysAgo: 29);

        await Rules.EnsureCanEditNoteAsync(block);
        (await LoadAsync(block.Id)).Id.ShouldBe(block.Id);
    }

    [Fact]
    public async Task Should_Refuse_Their_Own_Block_After_Thirty_Days()
    {
        var block = await SeedBlockAsync(CurrentUserId, daysAgo: 31);

        await Should.ThrowAsync<AbpAuthorizationException>(() => Rules.EnsureCanEditNoteAsync(block));
        await Should.ThrowAsync<AbpAuthorizationException>(() => LoadAsync(block.Id));
    }

    [Fact]
    public async Task Should_Hide_Another_Members_Block()
    {
        var block = await SeedBlockAsync(OtherUserId, daysAgo: 1);

        (await Should.ThrowAsync<EntityNotFoundException>(() => Rules.EnsureCanEditNoteAsync(block))).Id.ShouldBe(block.Id);
        await Should.ThrowAsync<EntityNotFoundException>(() => LoadAsync(block.Id));
    }
}

/// <summary>A team manager who cannot see other members' activity (Tracker.Team.Manage without ViewAll): they may save
/// another member's note, but no suggestion is made from content they cannot read.</summary>
public abstract class ActivityNoteRulesManagerWithoutViewAllTests<TStartupModule> : ActivityNoteRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll);
    }

    [Fact]
    public async Task Should_Allow_Editing_But_Not_Suggesting_For_Another_Members_Block()
    {
        var block = await SeedBlockAsync(OtherUserId, daysAgo: 1);

        await Rules.EnsureCanEditNoteAsync(block);
        (await Should.ThrowAsync<EntityNotFoundException>(() => LoadAsync(block.Id))).Id.ShouldBe(block.Id);
    }

    [Fact]
    public async Task Should_Allow_Their_Own_Old_Block()
    {
        var block = await SeedBlockAsync(CurrentUserId, daysAgo: 60);

        (await LoadAsync(block.Id)).Id.ShouldBe(block.Id);
    }
}

/// <summary>A viewer: sees everyone's activity (ViewAll) but does not manage the team.</summary>
public abstract class ActivityNoteRulesViewerTests<TStartupModule> : ActivityNoteRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task Should_Refuse_Another_Members_Block()
    {
        var block = await SeedBlockAsync(OtherUserId, daysAgo: 1);

        await Should.ThrowAsync<AbpAuthorizationException>(() => Rules.EnsureCanEditNoteAsync(block));
        await Should.ThrowAsync<AbpAuthorizationException>(() => LoadAsync(block.Id));
    }
}
