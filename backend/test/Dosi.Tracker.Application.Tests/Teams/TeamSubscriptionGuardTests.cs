using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// A workspace whose subscription is no longer usable (past due, expired trial) keeps its data readable and can shrink
/// its team, but cannot grow: no new projects, members or invites (402 Tracker:SubscriptionInactive). Which subscription
/// states are usable is <see cref="ISubscriptionAccessChecker"/>'s business (tested on its own), so it is stubbed here.
/// </summary>
public abstract class TeamSubscriptionGuardTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.Replace(ServiceDescriptor.Singleton<ISubscriptionAccessChecker>(new LapsedSubscription()));
    }

    /// <summary>A project from before the subscription lapsed, the caller its Admin and <paramref name="memberId"/> a worker.</summary>
    private async Task<Guid> SeedProjectAsync(Guid memberId)
    {
        var projectId = GuidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, TrackerTestConsts.WorkspaceId, "Existing"));
            var members = GetRequiredService<IRepository<ProjectMember, Guid>>();
            await members.InsertAsync(new ProjectMember(GuidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, CurrentUserId, ProjectMemberRoles.Admin));
            await members.InsertAsync(new ProjectMember(GuidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, memberId));
        });
        return projectId;
    }

    [Fact]
    public async Task The_Team_Cannot_Grow()
    {
        var member = await CreateUserAsync("member");
        var projectId = await SeedProjectAsync(member.Id);
        var newcomer = await CreateUserAsync("newcomer");

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, newcomer.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.InviteMemberAsync(
                new InviteMemberDto { ProjectId = projectId, Email = "late@acme.test" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);
        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("late@acme.test")))
            .ShouldBeNull();

        (await Should.ThrowAsync<BusinessException>(() => CreateProjectAsync("New")))
            .Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);
    }

    [Fact]
    public async Task A_Client_Cannot_Become_A_Tracked_Member()
    {
        // Turning a client into a worker takes a seat: growing the team by another way.
        var client = await CreateUserAsync("client");
        var projectId = GuidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, TrackerTestConsts.WorkspaceId, "Existing"));
            await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                new ProjectMember(GuidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, client.Id, ProjectMemberRoles.Client));
        });

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, client.Id, new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker })))
            .Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);

        (await GetMembershipsOfUserAsync(client.Id)).ShouldHaveSingleItem().Role.ShouldBe(ProjectMemberRoles.Client);
    }

    [Fact]
    public async Task The_Team_Can_Still_Be_Read_Changed_And_Shrunk()
    {
        var member = await CreateUserAsync("member");
        var projectId = await SeedProjectAsync(member.Id);

        (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Count.ShouldBe(2);
        (await TeamAppService.UpdateMemberAsync(projectId, member.Id, new UpdateProjectMemberDto { Role = "Client" }))
            .Role.ShouldBe("Client");

        await TeamAppService.RemoveFromWorkspaceAsync(member.Id);
        (await GetMembershipsOfUserAsync(member.Id)).ShouldBeEmpty();
    }

    private sealed class LapsedSubscription : ISubscriptionAccessChecker
    {
        public Task<bool> IsUsableAsync() => Task.FromResult(false);
    }
}
