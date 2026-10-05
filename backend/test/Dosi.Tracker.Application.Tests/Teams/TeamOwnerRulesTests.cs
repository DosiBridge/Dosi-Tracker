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
using Volo.Abp.PermissionManagement;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The owner-only rules, as a team lead meets them. The caller holds the Identity "manager" role — and with it
/// Tracker.Team.Manage — but is not a workspace owner: they may run the team, but not hand out manager rights, set pay,
/// touch an owner's memberships or change their own.
/// </summary>
public abstract class TeamOwnerRulesTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string? CallerRole => TrackerRoles.Manager;

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
        (await IsInRoleAsync(user.Id, TrackerRoles.Manager)).ShouldBeFalse();
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

        back.UserCreated.ShouldBeFalse();
        back.Member.HourlyRate.ShouldBe(40m);
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
        (await FindUserAsync(leaver.Id))!.IsActive.ShouldBeFalse();
    }

    [Fact]
    public async Task A_Manager_Cannot_Reset_Or_Remove_An_Account_With_Rights_Beyond_Their_Own()
    {
        var permissions = GetRequiredService<IPermissionManager>();
        // An accountant the owner gave billing rights through a role of their own, and someone granted project deletion.
        await WithUnitOfWorkAsync(async () =>
        {
            (await GetRequiredService<IdentityRoleManager>().CreateAsync(
                new IdentityRole(GuidGenerator.Create(), "accounting"))).Succeeded.ShouldBeTrue();
            await permissions.SetForRoleAsync("accounting", TrackerPermissions.Billing.Default, true);
        });
        var accountant = await CreateUserAsync("accountant", "accounting");
        var deleter = await CreateUserAsync("deleter");
        await WithUnitOfWorkAsync(() => permissions.SetForUserAsync(deleter.Id, TrackerPermissions.Projects.Delete, true));

        foreach (var target in new[] { accountant.Id, deleter.Id })
        {
            (await Should.ThrowAsync<BusinessException>(() => TeamAppService.ResetPasswordAsync(target)))
                .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
            (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(target)))
                .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
            (await FindUserAsync(target))!.IsActive.ShouldBeTrue();
        }

        // Another lead has the same rights as the caller.
        var colleague = await CreateUserAsync("colleague", TrackerRoles.Manager);
        (await TeamAppService.ResetPasswordAsync(colleague.Id)).InitialPassword.ShouldNotBeNullOrWhiteSpace();
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
    public async Task A_Manager_Cannot_Invite_A_Manager_Or_With_A_Rate_And_Creates_No_Account_Trying()
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

        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("alt.account@acme.test")))
            .ShouldBeNull();

        var worker = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId, Email = "plain.worker@acme.test", Role = "Worker"
        });
        worker.UserCreated.ShouldBeTrue();
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
        var owner = await CreateUserAsync("owner", TrackerRoles.Admin);

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, owner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        // An owner's membership created by an owner (e.g. on a project they created).
        var ownerProject = Guid.NewGuid();
        var membership = await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(ownerProject, null, "Owner's project"));
            return await GetRequiredService<ProjectMembershipManager>().EnrollAsync(ownerProject, owner.Id, "Admin", 0m);
        });

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                ownerProject, owner.Id, new UpdateProjectMemberDto { Role = "Worker" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveMemberAsync(membership.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
    }

    [Fact]
    public async Task A_Manager_Can_Reset_A_Members_Password_But_Not_An_Owners_Or_Their_Own()
    {
        var worker = await CreateUserAsync("worker");
        var owner = await CreateUserAsync("owner", TrackerRoles.Admin);

        (await TeamAppService.ResetPasswordAsync(worker.Id)).InitialPassword.ShouldNotBeNullOrWhiteSpace();

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.ResetPasswordAsync(owner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.ResetPasswordAsync(CurrentUserId)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);
    }

    [Fact]
    public async Task Nobody_Can_Remove_An_Owner_Or_Themselves_From_The_Workspace()
    {
        var owner = await CreateUserAsync("owner", TrackerRoles.Admin);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(owner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(CurrentUserId)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        (await FindUserAsync(owner.Id))!.IsActive.ShouldBeTrue();
        (await FindUserAsync(CurrentUserId))!.IsActive.ShouldBeTrue();
    }
}
