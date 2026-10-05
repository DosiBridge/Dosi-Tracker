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
using Xunit;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Teams;

/// <summary>
/// What happens around a membership's life: removal, project deletion, leaving the workspace, password resets and
/// invites of people who already have an account — always with the Identity "manager" role and the seats in step.
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
                new Subscription(GuidGenerator.Create(), null, plan.Id) { Status = Subscription.ActiveStatus });
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
        (await IsInRoleAsync(lead.Id, TrackerRoles.Manager)).ShouldBeTrue(); // still Admin on B

        await TeamAppService.RemoveMemberAsync(onB.Id);
        (await IsInRoleAsync(lead.Id, TrackerRoles.Manager)).ShouldBeFalse();
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

        (await IsInRoleAsync(lead.Id, TrackerRoles.Manager)).ShouldBeFalse();
        (await IsInRoleAsync(twoHats.Id, TrackerRoles.Manager)).ShouldBeTrue(); // still Manager on "Kept"

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
    public async Task RemoveFromWorkspace_Should_Remove_Every_Membership_Deactivate_The_User_And_Free_The_Seat()
    {
        await SubscribeAsync("Free"); // 3 seats
        var projectA = await CreateProjectAsync("A");
        var projectB = await CreateProjectAsync("B");
        var leaving = await CreateUserAsync("leaving");
        var stays = await CreateUserAsync("stays");
        await AddAsync(projectA, leaving.Id, "Manager");
        await AddAsync(projectB, leaving.Id);
        await AddAsync(projectA, stays.Id);
        var newcomer = await CreateUserAsync("newcomer");
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectA, newcomer.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);

        await TeamAppService.RemoveFromWorkspaceAsync(leaving.Id);

        (await GetMembershipsOfUserAsync(leaving.Id)).ShouldBeEmpty();
        (await FindUserAsync(leaving.Id))!.IsActive.ShouldBeFalse();
        (await IsInRoleAsync(leaving.Id, TrackerRoles.Manager)).ShouldBeFalse();
        (await CountSeatsAsync()).ShouldBe(2);

        // The freed seat can be given to someone else.
        (await AddAsync(projectA, newcomer.Id)).UserId.ShouldBe(newcomer.Id);

        // The removed person shows up as inactive. Bringing them back takes a seat like anyone else's, and the plan is
        // full again: refused, and the account stays inactive.
        var row = (await TeamAppService.GetMembersAsync()).Items.Single(m => m.UserId == leaving.Id);
        row.IsActive.ShouldBeFalse();
        row.ProjectIds.ShouldBeEmpty();
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectB, leaving.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        (await FindUserAsync(leaving.Id))!.IsActive.ShouldBeFalse();
    }

    [Fact]
    public async Task An_Owner_Can_Bring_Back_A_Removed_Member_Whose_Email_Is_Not_Verified()
    {
        var projectId = await CreateProjectAsync();
        // Invited before invites marked the address as verified, then removed from the workspace.
        var returning = await CreateUserAsync("returning", email: "returning@acme.test", emailConfirmed: false);
        await AddAsync(projectId, returning.Id, hourlyRate: 22m);
        await TeamAppService.RemoveFromWorkspaceAsync(returning.Id);

        // An invite never attaches to an unverified address...
        await Should.ThrowAsync<AbpValidationException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "returning@acme.test"
        }));

        // ...but the owner can add them back from the team list: the account is active again, at the rate it had.
        var member = await AddAsync(projectId, returning.Id, hourlyRate: 22m);

        member.UserId.ShouldBe(returning.Id);
        member.HourlyRate.ShouldBe(22m);
        (await FindUserAsync(returning.Id))!.IsActive.ShouldBeTrue();
        (await GetRatesAsync(projectId, returning.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(22m);
    }

    [Fact]
    public async Task RemoveFromWorkspace_Should_Take_A_Manager_Role_No_Membership_Backs()
    {
        // A lead whose "manager" role was given directly in Identity (or left over from before memberships kept it in
        // step), while their only membership is as a worker.
        var projectId = await CreateProjectAsync();
        var lead = await CreateUserAsync("lead", TrackerRoles.Manager, emailConfirmed: true, email: "lead@acme.test");
        await AddAsync(projectId, lead.Id);

        await TeamAppService.RemoveFromWorkspaceAsync(lead.Id);

        (await IsInRoleAsync(lead.Id, TrackerRoles.Manager)).ShouldBeFalse();

        // Inviting them back as a worker does not bring the role back.
        await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "lead@acme.test" });
        (await FindUserAsync(lead.Id))!.IsActive.ShouldBeTrue();
        (await IsInRoleAsync(lead.Id, TrackerRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task RemoveFromWorkspace_Should_Reject_An_Unknown_User()
    {
        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(Guid.NewGuid())))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
    }

    [Fact]
    public async Task ResetPassword_Should_Issue_A_OneTime_Password_And_Unlock_The_Account()
    {
        var user = await CreateUserAsync("forgetful");
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var stored = await userManager.GetByIdAsync(user.Id);
            (await userManager.AddPasswordAsync(stored, "Forgotten1!")).Succeeded.ShouldBeTrue();
            (await userManager.SetLockoutEnabledAsync(stored, true)).Succeeded.ShouldBeTrue();
            (await userManager.SetLockoutEndDateAsync(stored, DateTimeOffset.UtcNow.AddMinutes(5))).Succeeded.ShouldBeTrue();
        });

        var result = await TeamAppService.ResetPasswordAsync(user.Id);

        result.InitialPassword.Length.ShouldBe(20);
        (await CheckPasswordAsync(user.Id, result.InitialPassword)).ShouldBeTrue();
        (await CheckPasswordAsync(user.Id, "Forgotten1!")).ShouldBeFalse();

        var stored = (await FindUserAsync(user.Id))!;
        stored.ShouldChangePasswordOnNextLogin.ShouldBeTrue();
        (stored.LockoutEnd == null || stored.LockoutEnd <= DateTimeOffset.UtcNow).ShouldBeTrue();

        // Every reset gives a different password.
        (await TeamAppService.ResetPasswordAsync(user.Id)).InitialPassword.ShouldNotBe(result.InitialPassword);
    }

    [Fact]
    public async Task ResetPassword_Should_Work_For_An_Account_Without_Lockout()
    {
        var user = await CreateUserAsync("nolockout");
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.SetLockoutEnabledAsync(await userManager.GetByIdAsync(user.Id), false)).Succeeded.ShouldBeTrue();
        });

        var result = await TeamAppService.ResetPasswordAsync(user.Id);

        (await CheckPasswordAsync(user.Id, result.InitialPassword)).ShouldBeTrue();
        (await FindUserAsync(user.Id))!.LockoutEnabled.ShouldBeFalse();
    }

    [Fact]
    public async Task An_Owner_Can_Reset_Another_Owners_Password_But_Not_Remove_Them()
    {
        var coOwner = await CreateUserAsync("coowner", TrackerRoles.Admin);

        (await TeamAppService.ResetPasswordAsync(coOwner.Id)).InitialPassword.ShouldNotBeNullOrWhiteSpace();

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.RemoveFromWorkspaceAsync(coOwner.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
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
    public async Task Invite_Should_Turn_An_Address_That_Is_Not_A_Valid_User_Name_Into_One()
    {
        var projectId = await CreateProjectAsync();
        await CreateUserAsync("obrien", email: "obrien@elsewhere.test");

        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "o'brien@acme.test"
        });

        result.UserCreated.ShouldBeTrue();
        result.Member.UserName.ShouldBe("obrien2"); // "obrien" is taken
        result.Member.Email.ShouldBe("o'brien@acme.test");

        // They sign in with the address; the token endpoint looks the account up by email.
        var invitee = await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("o'brien@acme.test"));
        invitee.ShouldNotBeNull();
        invitee.Id.ShouldBe(result.Member.UserId);
    }

    [Fact]
    public async Task Invite_Should_Not_Attach_To_An_Account_With_An_Unverified_Email()
    {
        var projectId = await CreateProjectAsync();
        // A worker who set their own profile email to the address of a lead about to be invited.
        var squatter = await CreateUserAsync("squatter", email: "new.lead@acme.test", emailConfirmed: false);

        var exception = await Should.ThrowAsync<AbpValidationException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "new.lead@acme.test",
            Role = "Manager"
        }));

        exception.ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(InviteMemberDto.Email)));
        (await GetMembershipsOfUserAsync(squatter.Id)).ShouldBeEmpty();
        (await IsInRoleAsync(squatter.Id, TrackerRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task A_Changed_Email_Is_No_Longer_Verified_For_Invites()
    {
        var projectA = await CreateProjectAsync("A");
        var first = await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectA, Email = "kim@acme.test" });

        // Kim edits her profile email (ABP's profile endpoint goes through SetEmailAsync).
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var kim = await userManager.GetByIdAsync(first.Member.UserId);
            (await userManager.SetEmailAsync(kim, "boss@acme.test")).Succeeded.ShouldBeTrue();
        });

        var projectB = await CreateProjectAsync("B");
        await Should.ThrowAsync<AbpValidationException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectB,
            Email = "boss@acme.test"
        }));
    }

    [Fact]
    public async Task Inviting_A_Removed_Member_Again_Should_Bring_The_Account_Back_With_A_New_Password()
    {
        var projectId = await CreateProjectAsync();
        var first = await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "back@acme.test" });
        await TeamAppService.RemoveFromWorkspaceAsync(first.Member.UserId);

        var again = await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "back@acme.test" });

        again.UserCreated.ShouldBeFalse();
        again.Member.UserId.ShouldBe(first.Member.UserId);
        again.InitialPassword.ShouldNotBeNullOrWhiteSpace();
        again.InitialPassword.ShouldNotBe(first.InitialPassword);
        (await CheckPasswordAsync(first.Member.UserId, again.InitialPassword!)).ShouldBeTrue();

        var user = (await FindUserAsync(first.Member.UserId))!;
        user.IsActive.ShouldBeTrue();
        user.ShouldChangePasswordOnNextLogin.ShouldBeTrue();
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
