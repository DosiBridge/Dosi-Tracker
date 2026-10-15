using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Identity;
using Dosi.Tracker.SaaS;
using Microsoft.AspNetCore.Identity;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Validation;
using Dosi.Tracker.Workspaces;
using Xunit;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Teams;

/// <summary>
/// What happens around a membership's life: removal, project deletion, removal from the workspace and invites of people
/// who already have an account — always with the workspace manager role and the seats in step.
/// </summary>
public abstract class TeamMembershipLifecycleTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private Task<int> CountSeatsAsync()
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<InvoiceGenerator>().CountSeatsAsync());
    }

    private async Task SubscribeAsync(string planName)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var plan = await GetRequiredService<IRepository<Plan, Guid>>().FirstAsync(p => p.Name == planName);
            await GetRequiredService<IRepository<Subscription, Guid>>().InsertAsync(
                new Subscription(GuidGenerator.Create(), TrackerTestConsts.WorkspaceId, plan.Id) { Status = Subscription.ActiveStatus });
        });
    }

    private Task<bool> CheckPasswordAsync(Guid userId, string password)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            return await userManager.CheckPasswordAsync(await userManager.GetByIdAsync(userId), password);
        });
    }

    [Fact]
    public async Task RemoveMember_Should_Revoke_The_Manager_Role_With_The_Last_Manager_Membership()
    {
        var projectA = await CreateProjectAsync("A");
        var projectB = await CreateProjectAsync("B");
        var lead = await CreateUserAsync("lead");
        var onA = await AddAsync(projectA, lead.Id, "Manager");
        var onB = await AddAsync(projectB, lead.Id, "Admin");

        await TeamAppService.RemoveMemberAsync(onA.Id);
        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeTrue(); // still Admin on B

        await TeamAppService.RemoveMemberAsync(onB.Id);
        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task Deleting_A_Project_Should_Remove_Its_Memberships_Free_Their_Seats_And_Revoke_Unused_Manager_Roles()
    {
        var doomed = await CreateProjectAsync("Doomed");
        var kept = await CreateProjectAsync("Kept");
        var lead = await CreateUserAsync("lead");
        var twoHats = await CreateUserAsync("twohats");
        var worker = await CreateUserAsync("worker");
        await AddAsync(doomed, lead.Id, "Manager", 30m);
        await AddAsync(doomed, twoHats.Id, "Admin");
        await AddAsync(kept, twoHats.Id, "Manager");
        await AddAsync(doomed, worker.Id);
        (await CountSeatsAsync()).ShouldBe(4); // the caller, lead, twohats, worker

        await ProjectAppService.DeleteAsync(doomed);

        (await GetMembershipsOfUserAsync(lead.Id)).ShouldBeEmpty();
        (await GetMembershipsOfUserAsync(worker.Id)).ShouldBeEmpty();
        (await GetMembershipsOfUserAsync(twoHats.Id)).ShouldHaveSingleItem().ProjectId.ShouldBe(kept);
        (await CountSeatsAsync()).ShouldBe(2); // the caller and twohats, both on "Kept"

        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeFalse();
        (await IsInRoleAsync(twoHats.Id, WorkspaceRoles.Manager)).ShouldBeTrue(); // still Manager on "Kept"

        // Payroll of the time already tracked on the deleted project still finds the rate.
        (await GetRatesAsync(doomed, lead.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(30m);
    }

    [Fact]
    public async Task Deleting_A_Project_Twice_Should_Be_Harmless()
    {
        var projectId = await CreateProjectAsync();

        await ProjectAppService.DeleteAsync(projectId);
        await ProjectAppService.DeleteAsync(projectId);

        (await GetMembershipsOfUserAsync(CurrentUserId)).ShouldBeEmpty();
    }

    [Fact]
    public async Task RemoveFromWorkspace_Should_End_The_Membership_Remove_Project_Memberships_And_Free_The_Seat()
    {
        await SubscribeAsync("Free"); // 3 seats
        var projectA = await CreateProjectAsync("A");
        var projectB = await CreateProjectAsync("B");
        var leaving = await CreateUserAsync("leaving", email: "leaving@acme.test");
        var stays = await CreateUserAsync("stays");
        await AddAsync(projectA, leaving.Id, "Manager");
        await AddAsync(projectB, leaving.Id);
        await AddAsync(projectA, stays.Id);
        var newcomer = await CreateUserAsync("newcomer");
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectA, newcomer.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);

        await TeamAppService.RemoveFromWorkspaceAsync(leaving.Id);

        (await GetMembershipsOfUserAsync(leaving.Id)).ShouldBeEmpty();
        (await IsActiveMemberAsync(leaving.Id)).ShouldBeFalse();
        (await ServiceProvider.FindWorkspaceMemberAsync(leaving.Id))!.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        (await CountSeatsAsync()).ShouldBe(2);
        (await TeamAppService.GetMembersAsync()).Items.ShouldNotContain(m => m.UserId == leaving.Id);

        // The account itself is untouched: still active, same password, same security stamp.
        var account = (await FindUserAsync(leaving.Id))!;
        account.IsActive.ShouldBeTrue();
        account.ShouldChangePasswordOnNextLogin.ShouldBeFalse();
        account.SecurityStamp.ShouldBe(leaving.SecurityStamp);

        // The freed seat can be given to someone else; the removed person can't be added to a project directly.
        (await AddAsync(projectA, newcomer.Id)).UserId.ShouldBe(newcomer.Id);
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectB, leaving.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
    }

    [Fact]
    public async Task Inviting_A_Removed_Member_Again_Should_Bring_Them_Back_Without_Touching_The_Account()
    {
        var projectId = await CreateProjectAsync();
        var returning = await CreateUserAsync("returning", email: "returning@acme.test");
        await AddAsync(projectId, returning.Id, hourlyRate: 22m);
        await TeamAppService.RemoveFromWorkspaceAsync(returning.Id);

        var again = await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "returning@acme.test", HourlyRate = 22m });

        again.Added.ShouldBeTrue();
        again.Member!.UserId.ShouldBe(returning.Id);
        again.Member.HourlyRate.ShouldBe(22m); // the rate it had
        (await IsInRoleAsync(returning.Id, WorkspaceRoles.Member)).ShouldBeTrue();
        (await FindUserAsync(returning.Id))!.SecurityStamp.ShouldBe(returning.SecurityStamp);
        (await GetRatesAsync(projectId, returning.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(22m);
    }

    [Fact]
    public async Task RemoveFromWorkspace_Should_Take_A_Manager_Role_No_Membership_Backs()
    {
        // A lead whose "manager" role was given directly in Identity (or left over from before memberships kept it in
        // step), while their only membership is as a worker.
        var projectId = await CreateProjectAsync();
        var lead = await CreateUserAsync("lead", WorkspaceRoles.Manager, emailConfirmed: true, email: "lead@acme.test");
        await AddAsync(projectId, lead.Id);

        await TeamAppService.RemoveFromWorkspaceAsync(lead.Id);

        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeFalse();

        // Inviting them back as a worker does not bring the role back.
        await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "lead@acme.test" });
        (await FindUserAsync(lead.Id))!.IsActive.ShouldBeTrue();
        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task RemoveFromWorkspace_Should_Reject_An_Unknown_User()
    {
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(Guid.NewGuid())))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
    }

    [Fact]
    public async Task No_One_Should_Remove_An_Owner_From_The_Workspace()
    {
        var coOwner = await CreateUserAsync("coowner", WorkspaceRoles.Owner);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(coOwner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        (await IsInRoleAsync(coOwner.Id, WorkspaceRoles.Owner)).ShouldBeTrue();
    }

    [Fact]
    public async Task An_Owner_Should_Change_Workspace_Roles_But_Never_Leave_The_Workspace_Without_An_Owner()
    {
        var member = await CreateUserAsync("rising");

        (await TeamAppService.UpdateWorkspaceRoleAsync(member.Id, "Manager")).Role.ShouldBe(WorkspaceRoles.Manager);
        (await TeamAppService.UpdateWorkspaceRoleAsync(member.Id, WorkspaceRoles.Owner)).IsOwner.ShouldBeTrue();
        (await TeamAppService.UpdateWorkspaceRoleAsync(member.Id, WorkspaceRoles.Member)).Role.ShouldBe(WorkspaceRoles.Member);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateWorkspaceRoleAsync(CurrentUserId, WorkspaceRoles.Member)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);
        await Should.ThrowAsync<AbpValidationException>(() => TeamAppService.UpdateWorkspaceRoleAsync(member.Id, "emperor"));
    }

    [Fact]
    public async Task An_Owner_Cannot_Change_Their_Own_Role_Or_Rate_But_May_Leave_A_Project()
    {
        var projectId = await CreateProjectAsync();

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, CurrentUserId, new UpdateProjectMemberDto { Role = "Admin", HourlyRate = 50m })))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, CurrentUserId, new UpdateProjectMemberDto { Role = "Worker" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);

        var own = (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Single(m => m.UserId == CurrentUserId);
        await TeamAppService.RemoveMemberAsync(own.Id);
        (await TeamAppService.GetProjectMembersAsync(projectId)).Items.ShouldBeEmpty();

        // Joining as a plain worker is fine; joining with a role or a rate would be assigning it to yourself.
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, CurrentUserId, ProjectMemberRoles.Manager)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, CurrentUserId, hourlyRate: 10m)))
            .Code.ShouldBe(TrackerDomainErrorCodes.CannotModifySelf);
        (await AddAsync(projectId, CurrentUserId)).Role.ShouldBe("Worker");
    }

    [Fact]
    public async Task Signing_Up_With_An_Address_That_Is_Not_A_Valid_User_Name_Should_Get_One()
    {
        await CreateUserAsync("obrien", email: "obrien@elsewhere.test");

        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            WorkspaceName = "obrien-co",
            Email = "o'brien@acme.test",
            Password = "1q2w3E*obrien"
        });

        result.UserName.ShouldBe("obrien2"); // "obrien" is taken
        var account = await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("o'brien@acme.test"));
        account.ShouldNotBeNull().UserName.ShouldBe("obrien2");
    }

    [Fact]
    public async Task Users_Should_Not_Change_Their_Email_Which_Invitations_Are_Matched_By()
    {
        var value = await GetRequiredService<Volo.Abp.Settings.ISettingProvider>()
            .GetOrNullAsync(Volo.Abp.Identity.Settings.IdentitySettingNames.User.IsEmailUpdateEnabled);

        value.ShouldBe("False");
    }

    [Fact]
    public async Task Turning_A_Client_Into_A_Tracked_Member_Needs_A_Free_Seat()
    {
        await SubscribeAsync("Free"); // 3 seats
        var projectId = await CreateProjectAsync();
        await AddAsync(projectId, (await CreateUserAsync("w1")).Id);
        await AddAsync(projectId, (await CreateUserAsync("w2")).Id);
        var client = await CreateUserAsync("client");
        await AddAsync(projectId, client.Id, ProjectMemberRoles.Client); // no seat needed

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.UpdateMemberAsync(
                projectId, client.Id, new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker })))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
    }
}
