using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The owner-only rules, as a team lead meets them. The caller is a workspace manager — and with it holds
/// Tracker.Team.Manage — but not an owner: they may run the team, but not hand out manager rights, set pay, change
/// workspace roles, touch an owner or another manager, or change their own.
/// </summary>
public abstract class TeamOwnerRulesTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string? CallerRole => WorkspaceRoles.Manager;

    [Theory]
    [InlineData("Admin")]
    [InlineData("Manager")]
    [InlineData(" manager ")]
    public async Task A_Manager_Cannot_Add_Someone_As_Admin_Or_Manager(string role)
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        var exception = await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, user.Id, role));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await GetMembershipsOfUserAsync(user.Id)).ShouldBeEmpty();
        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task A_Manager_Can_Add_Workers_And_Clients_Without_A_Rate()
    {
        var projectId = await CreateProjectAsync();

        (await AddAsync(projectId, (await CreateUserAsync("worker")).Id)).Role.ShouldBe("Worker");
        (await AddAsync(projectId, (await CreateUserAsync("client")).Id, "client")).Role.ShouldBe("Client");
    }

    [Fact]
    public async Task A_Manager_Cannot_Set_A_Rate()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, user.Id, hourlyRate: 25m)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        await AddAsync(projectId, user.Id);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, user.Id, new UpdateProjectMemberDto { Role = "Worker", HourlyRate = 25m })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await GetRatesAsync(projectId, user.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(0m);
    }

    [Fact]
    public async Task A_Manager_Cannot_Reprice_A_Member_By_Removing_And_Adding_Them_Again()
    {
        var projectId = await CreateProjectAsync();
        var worker = await CreateUserAsync("worker");
        // The owner pays the worker 25/h on the project.
        var membership = await WithUnitOfWorkAsync(() =>
            GetRequiredService<ProjectMembershipManager>().EnrollAsync(projectId, worker.Id, ProjectMemberRoles.Worker, 25m));

        await TeamAppService.RemoveMemberAsync(membership.Id);
        var readded = await AddAsync(projectId, worker.Id); // no rate given: the recorded one stays

        readded.HourlyRate.ShouldBe(25m);
        (await GetRatesAsync(projectId, worker.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(25m);

        // Asking for another rate is setting one, which is the owner's to do; asking for the same one is no change.
        await TeamAppService.RemoveMemberAsync(readded.Id);
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, worker.Id, hourlyRate: 10m)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await AddAsync(projectId, worker.Id, hourlyRate: 25m)).HourlyRate.ShouldBe(25m);
        (await GetRatesAsync(projectId, worker.Id)).ShouldHaveSingleItem();
    }

    [Fact]
    public async Task A_Manager_Cannot_Reprice_A_Member_By_Removing_Them_From_The_Workspace_And_Inviting_Them_Again()
    {
        var projectId = await CreateProjectAsync();
        var worker = await CreateUserAsync("worker", emailConfirmed: true, email: "worker@acme.test");
        await WithUnitOfWorkAsync(() =>
            GetRequiredService<ProjectMembershipManager>().EnrollAsync(projectId, worker.Id, ProjectMemberRoles.Worker, 40m));

        await TeamAppService.RemoveFromWorkspaceAsync(worker.Id);
        var back = await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "worker@acme.test" });

        back.Added.ShouldBeFalse();
        await AcceptAsync(worker.Id, back);

        (await GetMembershipsOfUserAsync(worker.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(40m);
        (await GetRatesAsync(projectId, worker.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(40m);
    }

    [Fact]
    public async Task A_Manager_Cannot_Bring_Back_Someone_Removed_From_The_Workspace_By_Id()
    {
        var projectId = await CreateProjectAsync();
        var leaver = await CreateUserAsync("leaver");
        await AddAsync(projectId, leaver.Id);
        await TeamAppService.RemoveFromWorkspaceAsync(leaver.Id);

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, leaver.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
        (await FindUserAsync(leaver.Id))!.IsActive.ShouldBeTrue(); // the account itself is untouched
    }

    [Fact]
    public async Task A_Manager_Can_Remove_Members_But_Not_Other_Managers_Or_Change_Workspace_Roles()
    {
        var worker = await CreateUserAsync("worker");
        var colleague = await CreateUserAsync("colleague", WorkspaceRoles.Manager);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(colleague.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await IsInRoleAsync(colleague.Id, WorkspaceRoles.Manager)).ShouldBeTrue();

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateWorkspaceRoleAsync(worker.Id, WorkspaceRoles.Manager)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        await TeamAppService.RemoveFromWorkspaceAsync(worker.Id);
        (await IsActiveMemberAsync(worker.Id)).ShouldBeFalse();
    }

    [Fact]
    public async Task A_Manager_Cannot_Promote_But_Can_Relabel_And_Demote()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");
        await AddAsync(projectId, user.Id);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, user.Id, new UpdateProjectMemberDto { Role = "Manager" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await TeamAppService.UpdateMemberAsync(projectId, user.Id, new UpdateProjectMemberDto { Role = "Client" }))
            .Role.ShouldBe("Client");
    }

    [Fact]
    public async Task A_Manager_Cannot_Invite_A_Manager_Or_Owner_Or_With_A_Rate()
    {
        var projectId = await CreateProjectAsync();

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
            {
                ProjectId = projectId, Email = "alt.account@acme.test", Role = "Admin"
            })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
            {
                ProjectId = projectId, Email = "alt.account@acme.test", Role = "Worker", HourlyRate = 100m
            })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
            {
                Email = "alt.account@acme.test", WorkspaceRole = WorkspaceRoles.Owner
            })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await TeamAppService.GetInvitationsAsync()).Items.ShouldBeEmpty();

        var worker = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId, Email = "plain.worker@acme.test", Role = "Worker"
        });
        worker.InvitePath.ShouldNotBeNull();
    }

    [Fact]
    public async Task Nobody_Can_Change_Their_Own_Role_Or_Rate()
    {
        var projectId = await CreateProjectAsync(); // the caller is its Admin

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, CurrentUserId, new UpdateProjectMemberDto { Role = "Admin", HourlyRate = 10000m })))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, CurrentUserId, new UpdateProjectMemberDto { Role = "Worker" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        // Leaving a project is up to an owner as well.
        var own = (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Single(m => m.UserId == CurrentUserId);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveMemberAsync(own.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        (await GetRatesAsync(projectId, CurrentUserId)).ShouldHaveSingleItem().HourlyRate.ShouldBe(0m);
    }

    [Fact]
    public async Task A_Manager_Cannot_Touch_An_Owners_Memberships()
    {
        var projectId = await CreateProjectAsync();
        var owner = await CreateUserAsync("owner", WorkspaceRoles.Owner);

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, owner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        // An owner's membership created by an owner (e.g. on a project they created).
        var ownerProject = Guid.NewGuid();
        var membership = await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(ownerProject, TrackerTestConsts.WorkspaceId, "Owner's project"));
            return await GetRequiredService<ProjectMembershipManager>().EnrollAsync(ownerProject, owner.Id, "Admin", 0m);
        });

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                ownerProject, owner.Id, new UpdateProjectMemberDto { Role = "Worker" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveMemberAsync(membership.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
    }

    [Fact]
    public async Task Nobody_Can_Remove_An_Owner_Or_Themselves_From_The_Workspace()
    {
        var owner = await CreateUserAsync("owner", WorkspaceRoles.Owner);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(owner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(CurrentUserId)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        (await FindUserAsync(owner.Id))!.IsActive.ShouldBeTrue();
        (await FindUserAsync(CurrentUserId))!.IsActive.ShouldBeTrue();
    }
}
