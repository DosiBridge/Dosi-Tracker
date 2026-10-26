using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>The team service as a workspace owner uses it (see <see cref="TeamTestBase{TStartupModule}"/>).</summary>
public abstract class TeamAppServiceTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;

    protected TeamAppServiceTests()
    {
        _notificationRepository = GetRequiredService<IRepository<Notification, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
    }

    [Fact]
    public async Task Should_Add_And_List_Project_Members()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        var member = await AddAsync(projectId, user.Id);

        member.ProjectId.ShouldBe(projectId);
        member.UserId.ShouldBe(user.Id);
        member.Role.ShouldBe("Worker");

        var members = await TeamAppService.GetProjectMembersAsync(projectId);

        // Creator is auto-enrolled as Admin, plus the member just added.
        members.Items.Count.ShouldBe(2);
        members.Items.ShouldContain(m => m.UserId == CurrentUserId && m.Role == "Admin");
        members.Items.ShouldContain(m => m.UserId == user.Id && m.Role == "Worker");
    }

    [Fact]
    public async Task Project_Members_Should_Carry_The_Names_And_Emails()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("alice");
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var stored = await userManager.GetByIdAsync(user.Id);
            stored.Surname = "Liddell";
            (await userManager.UpdateAsync(stored)).Succeeded.ShouldBeTrue();
        });

        var added = await AddAsync(projectId, user.Id);
        added.UserName.ShouldBe("alice");
        added.Email.ShouldBe("alice@acme.test");

        var row = (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Single(m => m.UserId == user.Id);
        row.UserName.ShouldBe("alice");
        row.Name.ShouldBe("ALICE");
        row.Surname.ShouldBe("Liddell");
        row.Email.ShouldBe("alice@acme.test");
        row.CreationTime.ShouldNotBe(default);
    }

    [Fact]
    public async Task AddMember_Should_Reject_Duplicate_Membership()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        await AddAsync(projectId, user.Id);

        var exception = await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, user.Id, "Admin"));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.DuplicateProjectMember);
    }

    [Fact]
    public async Task AddMember_Should_Reject_Unknown_Project()
    {
        var user = await CreateUserAsync("worker");

        await Should.ThrowAsync<EntityNotFoundException>(() => AddAsync(Guid.NewGuid(), user.Id));
    }

    [Fact]
    public async Task AddMember_Should_Reject_A_User_Id_That_Is_Not_In_The_Workspace()
    {
        var projectId = await CreateProjectAsync();

        var unknown = await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, Guid.NewGuid()));
        unknown.Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);

        // An account that is not a member of this workspace (only of another one) can't be added.
        var stranger = await CreateUserAsync("stranger", member: false);
        await ServiceProvider.AddWorkspaceMemberAsync(stranger.Id, WorkspaceRoles.Member, Guid.NewGuid());
        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, stranger.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);

        (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Count.ShouldBe(1); // only the creator
    }

    [Fact]
    public async Task AddMember_Should_Refuse_Someone_Removed_From_The_Workspace()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("gone");
        await TeamAppService.RemoveFromWorkspaceAsync(user.Id);

        (await Should.ThrowAsync<BusinessException>(() => AddAsync(projectId, user.Id)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
    }

    [Fact]
    public async Task RemoveMember_Should_Remove_The_Membership()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        var member = await AddAsync(projectId, user.Id);

        await TeamAppService.RemoveMemberAsync(member.Id);

        var members = await TeamAppService.GetProjectMembersAsync(projectId);
        members.Items.Count.ShouldBe(1); // only the auto-enrolled creator remains
        members.Items[0].UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task RemoveMember_Should_Reject_An_Unknown_Membership()
    {
        await Should.ThrowAsync<EntityNotFoundException>(() => TeamAppService.RemoveMemberAsync(Guid.NewGuid()));
    }

    [Fact]
    public async Task AddMember_Should_Notify_The_New_Member()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        await AddAsync(projectId, user.Id);

        await WithUnitOfWorkAsync(async () =>
        {
            var notifications = await _notificationRepository.GetListAsync(n => n.UserId == user.Id);
            notifications.Count.ShouldBe(1);
            notifications[0].Message.ShouldContain("Team Test Project");
            notifications[0].IsRead.ShouldBeFalse();
        });
    }

    [Fact]
    public async Task AddMember_Should_Persist_The_Hourly_Rate()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        var member = await AddAsync(projectId, user.Id, hourlyRate: 42.5m);

        member.HourlyRate.ShouldBe(42.5m);

        var members = await TeamAppService.GetProjectMembersAsync(projectId);
        members.Items.First(m => m.Id == member.Id).HourlyRate.ShouldBe(42.5m);
    }

    [Fact]
    public async Task AddMember_Should_Start_The_Rate_History()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");
        var before = DateTime.UtcNow.AddSeconds(-1);

        await AddAsync(projectId, user.Id, hourlyRate: 20m);

        var rate = (await GetRatesAsync(projectId, user.Id)).ShouldHaveSingleItem();
        rate.HourlyRate.ShouldBe(20m);
        rate.EffectiveFrom.ShouldBeGreaterThanOrEqualTo(before);
        rate.EffectiveFrom.ShouldBeLessThanOrEqualTo(DateTime.UtcNow.AddSeconds(1));

        // The creator's own (unpaid) membership has its history too.
        (await GetRatesAsync(projectId, CurrentUserId)).ShouldHaveSingleItem().HourlyRate.ShouldBe(0m);
    }

    [Fact]
    public async Task UpdateMember_Should_Append_A_Rate_Only_When_The_Rate_Changes()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");
        await AddAsync(projectId, user.Id, hourlyRate: 20m);

        await TeamAppService.UpdateMemberAsync(projectId, user.Id, new UpdateProjectMemberDto { Role = "Worker", HourlyRate = 30m });
        await TeamAppService.UpdateMemberAsync(projectId, user.Id, new UpdateProjectMemberDto { Role = "Client", HourlyRate = 30m });

        var rates = await GetRatesAsync(projectId, user.Id);
        rates.Select(r => r.HourlyRate).ShouldBe(new[] { 20m, 30m });
        rates[1].EffectiveFrom.ShouldBeGreaterThanOrEqualTo(rates[0].EffectiveFrom);
    }

    [Fact]
    public async Task Readding_A_Member_At_Their_Old_Rate_Should_Not_Duplicate_The_History()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");
        var member = await AddAsync(projectId, user.Id, hourlyRate: 25m);
        await TeamAppService.RemoveMemberAsync(member.Id);

        await AddAsync(projectId, user.Id, hourlyRate: 25m);
        (await GetRatesAsync(projectId, user.Id)).ShouldHaveSingleItem();

        // Rates are history: removing the member keeps them for the time already tracked.
        var again = (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Single(m => m.UserId == user.Id);
        await TeamAppService.RemoveMemberAsync(again.Id);
        (await GetRatesAsync(projectId, user.Id)).ShouldHaveSingleItem();
    }

    [Fact]
    public async Task Invite_Of_An_Unknown_Email_Should_Create_A_Pending_Invitation_And_No_Account()
    {
        var projectId = await CreateProjectAsync();

        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "new.worker@acme.test",
            Role = "Worker",
            HourlyRate = 30m
        });

        result.Added.ShouldBeFalse();
        result.Member.ShouldBeNull();
        result.WorkspaceRole.ShouldBe(WorkspaceRoles.Member);
        result.InvitationId.ShouldNotBeNull();
        result.InvitePath.ShouldNotBeNull().ShouldStartWith("/invite/");
        result.InvitePath!.Length.ShouldBeGreaterThan("/invite/".Length + 32);
        result.ExpiresAt!.Value.ShouldBeInRange(DateTime.UtcNow.AddDays(14).AddMinutes(-1), DateTime.UtcNow.AddDays(14).AddMinutes(1));

        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("new.worker@acme.test"))).ShouldBeNull();
        (await TeamAppService.GetProjectMembersAsync(projectId)).Items.Count.ShouldBe(1); // only the creator

        var pending = (await TeamAppService.GetInvitationsAsync()).Items.ShouldHaveSingleItem();
        pending.Email.ShouldBe("new.worker@acme.test");
        pending.ProjectId.ShouldBe(projectId);
        pending.ProjectRole.ShouldBe("Worker");

        // Only the hash of the token is stored.
        var token = result.InvitePath["/invite/".Length..];
        await WithUnitOfWorkAsync(async () =>
        {
            var stored = await GetRequiredService<IRepository<WorkspaceInvitation, Guid>>().GetAsync(result.InvitationId!.Value);
            stored.TokenHash.ShouldBe(WorkspaceInvitation.HashToken(token));
            stored.TokenHash.ShouldNotBe(token);
            stored.HourlyRate.ShouldBe(30m);
        });

        await TeamAppService.DeleteInvitationAsync(pending.Id);
        (await TeamAppService.GetInvitationsAsync()).Items.ShouldBeEmpty();
    }

    [Fact]
    public async Task Invite_Of_An_Existing_Account_Should_Create_A_Pending_Invitation_Not_A_Membership()
    {
        var projectA = await CreateProjectAsync();
        var existing = await CreateUserAsync("repeat.worker", email: "repeat.worker@acme.test", member: false);

        var first = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectA,
            Email = "REPEAT.worker@acme.test",
            Role = "Worker",
            HourlyRate = 30m
        });

        // Nobody joins a workspace without accepting, and the answer looks exactly like one for an unknown email.
        first.Added.ShouldBeFalse();
        first.Member.ShouldBeNull();
        first.WorkspaceRole.ShouldBe(WorkspaceRoles.Member);
        first.InvitationId.ShouldNotBeNull();
        first.InvitePath.ShouldNotBeNull().ShouldStartWith("/invite/");
        (await IsActiveMemberAsync(existing.Id)).ShouldBeFalse();
        (await GetMembershipsOfUserAsync(existing.Id)).ShouldBeEmpty();
        await WithUnitOfWorkAsync(async () =>
            (await _notificationRepository.GetListAsync(n => n.UserId == existing.Id)).ShouldBeEmpty());

        // Accepting with the link joins the workspace and the project at the invited rate.
        var accepted = await AcceptAsync(existing.Id, first);
        accepted.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
        accepted.Role.ShouldBe(WorkspaceRoles.Member);
        (await IsInRoleAsync(existing.Id, WorkspaceRoles.Member)).ShouldBeTrue();
        (await GetMembershipsOfUserAsync(existing.Id)).ShouldHaveSingleItem().HourlyRate.ShouldBe(30m);

        // Once a member, an invitation to another project adds them to it at once.
        var projectB = await CreateProjectAsync("Second Invite Project");
        var second = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectB,
            Email = "repeat.worker@acme.test",
            Role = "Admin"
        });

        second.Added.ShouldBeTrue();
        second.InvitePath.ShouldBeNull();
        second.Member!.UserId.ShouldBe(existing.Id);
        second.WorkspaceRole.ShouldBe(WorkspaceRoles.Manager);

        (await Should.ThrowAsync<BusinessException>(() => TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            Email = "repeat.worker@acme.test"
        }))).Code.ShouldBe(TrackerDomainErrorCodes.AlreadyWorkspaceMember);
    }

    [Fact]
    public async Task Invite_Should_Answer_The_Same_For_Accounts_That_Exist_Are_Inactive_Or_Do_Not_Exist()
    {
        var existing = await CreateUserAsync("outsider", member: false);
        var inactive = await CreateUserAsync("disabled", member: false);
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var stored = await userManager.GetByIdAsync(inactive.Id);
            stored.SetIsActive(false);
            (await userManager.UpdateAsync(stored)).Succeeded.ShouldBeTrue();
        });

        foreach (var email in new[] { existing.Email, inactive.Email, "nobody@acme.test" })
        {
            var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = email });

            result.Added.ShouldBeFalse();
            result.Member.ShouldBeNull();
            result.WorkspaceRole.ShouldBe(WorkspaceRoles.Member);
            result.InvitationId.ShouldNotBeNull();
            result.InvitePath.ShouldNotBeNull();
        }

        (await IsActiveMemberAsync(existing.Id)).ShouldBeFalse();
        (await IsActiveMemberAsync(inactive.Id)).ShouldBeFalse();
        (await TeamAppService.GetInvitationsAsync()).Items.Count.ShouldBe(3);
    }

    [Fact]
    public async Task Invite_Without_A_Project_Should_Only_Grant_The_Workspace_Once_Accepted()
    {
        var existing = await CreateUserAsync("solo", member: false);

        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            Email = existing.Email,
            WorkspaceRole = "manager"
        });

        result.Added.ShouldBeFalse();
        result.Member.ShouldBeNull();
        result.WorkspaceRole.ShouldBe(WorkspaceRoles.Manager);
        (await IsActiveMemberAsync(existing.Id)).ShouldBeFalse();

        await AcceptAsync(existing.Id, result);

        (await IsInRoleAsync(existing.Id, WorkspaceRoles.Manager)).ShouldBeTrue();
        (await GetMembershipsOfUserAsync(existing.Id)).ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Email_Squatter_Should_Never_Join_A_Workspace_That_Invites_The_Email()
    {
        // Anyone can sign up with any address: the "invitee" here registered it first, unverified.
        var projectId = await CreateProjectAsync();
        var squatter = await CreateUserAsync("future.hire", email: "future.hire@acme.test", member: false);

        var invitation = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "future.hire@acme.test",
            Role = "Manager",
            WorkspaceRole = "manager"
        });

        (await IsActiveMemberAsync(squatter.Id)).ShouldBeFalse();
        (await GetMembershipsOfUserAsync(squatter.Id)).ShouldBeEmpty();

        // An unverified email neither lists the invitation nor stands in for its token.
        var mine = await AsUserAsync(squatter.Id, service => service.GetMyInvitationsAsync());
        mine.EmailConfirmed.ShouldBeFalse();
        mine.Items.ShouldBeEmpty();

        (await Should.ThrowAsync<BusinessException>(() => AsUserAsync(squatter.Id,
                service => service.AcceptMyInvitationAsync(invitation.InvitationId!.Value))))
            .Code.ShouldBe(TrackerDomainErrorCodes.EmailNotConfirmed);
        (await Should.ThrowAsync<BusinessException>(() => AsUserAsync(squatter.Id, async service =>
            {
                await service.DeclineMyInvitationAsync(invitation.InvitationId!.Value);
                return true;
            })))
            .Code.ShouldBe(TrackerDomainErrorCodes.EmailNotConfirmed);

        (await IsActiveMemberAsync(squatter.Id)).ShouldBeFalse();
        (await TeamAppService.GetInvitationsAsync()).Items.ShouldHaveSingleItem().Id.ShouldBe(invitation.InvitationId!.Value);
    }

    [Fact]
    public async Task A_Verified_Invitee_Should_See_And_Accept_Their_Invitations_Without_The_Link()
    {
        var invitee = await CreateUserAsync("verified", emailConfirmed: true, member: false);
        var stranger = await CreateUserAsync("other.verified", emailConfirmed: true, member: false);

        var invitation = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            Email = "VERIFIED@acme.test",
            WorkspaceRole = "manager"
        });
        var declined = await TeamAppService.InviteMemberAsync(new InviteMemberDto { Email = stranger.Email });

        var mine = await AsUserAsync(invitee.Id, service => service.GetMyInvitationsAsync());
        mine.EmailConfirmed.ShouldBeTrue();
        var item = mine.Items.ShouldHaveSingleItem();
        item.Id.ShouldBe(invitation.InvitationId!.Value);
        item.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
        item.WorkspaceName.ShouldNotBeNullOrWhiteSpace();
        item.Role.ShouldBe(WorkspaceRoles.Manager);

        // Someone else's invitation is not found by id, so ids reveal nothing.
        (await Should.ThrowAsync<BusinessException>(() => AsUserAsync(invitee.Id,
                service => service.AcceptMyInvitationAsync(declined.InvitationId!.Value))))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        (await Should.ThrowAsync<BusinessException>(() => AsUserAsync(invitee.Id,
                service => service.AcceptMyInvitationAsync(Guid.NewGuid()))))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);

        var accepted = await AsUserAsync(invitee.Id, service => service.AcceptMyInvitationAsync(item.Id));
        accepted.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
        accepted.Role.ShouldBe(WorkspaceRoles.Manager);
        (await IsInRoleAsync(invitee.Id, WorkspaceRoles.Manager)).ShouldBeTrue();

        // A used invitation is gone from the list and cannot be used again.
        (await AsUserAsync(invitee.Id, service => service.GetMyInvitationsAsync())).Items.ShouldBeEmpty();
        (await Should.ThrowAsync<BusinessException>(() => AsUserAsync(invitee.Id,
                service => service.AcceptMyInvitationAsync(item.Id))))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvitationExpired);

        // The other verified invitee declines theirs by id: no membership, and the invitation is gone.
        await AsUserAsync(stranger.Id, async service =>
        {
            await service.DeclineMyInvitationAsync(declined.InvitationId!.Value);
            return true;
        });
        (await IsActiveMemberAsync(stranger.Id)).ShouldBeFalse();
        (await TeamAppService.GetInvitationsAsync()).Items.ShouldBeEmpty();
    }

    [Fact]
    public async Task AddMember_Should_Enforce_The_Plan_Seat_Limit()
    {
        // Free plan (seeded): 3 seats. Creator occupies the first seat automatically.
        var freePlan = await WithUnitOfWorkAsync(
            () => _planRepository.FirstAsync(p => p.Name == "Free"));
        await WithUnitOfWorkAsync(async () =>
        {
            await _subscriptionRepository.InsertAsync(
                new Subscription(GuidGenerator.Create(), TrackerTestConsts.WorkspaceId, freePlan.Id));
        });

        var projectId = await CreateProjectAsync();

        await AddAsync(projectId, (await CreateUserAsync("w1")).Id);
        await AddAsync(projectId, (await CreateUserAsync("w2")).Id);

        // Seat 4 must be rejected on the 3-seat plan.
        var exception = await Should.ThrowAsync<BusinessException>(
            async () => await AddAsync(projectId, (await CreateUserAsync("w3")).Id));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);

        // But an existing member (already occupying a seat) can join another project.
        var secondProject = await CreateProjectAsync("Second Project");
        var existingMemberId = (await TeamAppService.GetProjectMembersAsync(projectId))
            .Items.First(m => m.UserId != CurrentUserId).UserId;

        var again = await AddAsync(secondProject, existingMemberId);
        again.UserId.ShouldBe(existingMemberId);

        // A client follows progress and needs no seat.
        var client = await AddAsync(projectId, (await CreateUserAsync("client")).Id, ProjectMemberRoles.Client);
        client.Role.ShouldBe("Client");
    }

    [Theory]
    [InlineData("Admin")]
    [InlineData("manager")]
    [InlineData(" MANAGER ")]
    public async Task AddMember_As_Admin_Or_Manager_Should_Grant_The_Manager_Role(string role)
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("lead");

        var member = await AddAsync(projectId, user.Id, role);

        member.Role.ShouldBe(ProjectMemberRoles.Normalize(role)); // stored in its canonical spelling
        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeTrue();
    }

    [Theory]
    [InlineData("Worker")]
    [InlineData("Employee")]
    [InlineData("member")]
    [InlineData("Client")]
    public async Task AddMember_As_Worker_Should_Not_Grant_Any_Role(string role)
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("worker");

        await AddAsync(projectId, user.Id, role);

        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeFalse();
    }

    [Fact]
    public async Task Invite_As_Manager_Should_Grant_The_Manager_Role_Once_Accepted()
    {
        var projectId = await CreateProjectAsync();
        var lead = await CreateUserAsync("lead", email: "lead@acme.test", member: false);

        var result = await TeamAppService.InviteMemberAsync(new InviteMemberDto
        {
            ProjectId = projectId,
            Email = "lead@acme.test",
            Role = "Manager"
        });

        result.WorkspaceRole.ShouldBe(WorkspaceRoles.Manager);
        (await IsActiveMemberAsync(lead.Id)).ShouldBeFalse();

        await AcceptAsync(lead.Id, result);

        (await IsInRoleAsync(lead.Id, WorkspaceRoles.Manager)).ShouldBeTrue();
        (await GetMembershipsOfUserAsync(lead.Id)).ShouldHaveSingleItem().Role.ShouldBe("Manager");
    }

    [Fact]
    public async Task GetMembers_Should_List_The_Active_Members_With_Roles_And_Projects()
    {
        var projectId = await CreateProjectAsync();
        var lead = await CreateUserAsync("lead");
        var worker = await CreateUserAsync("worker");
        var client = await CreateUserAsync("client");
        await AddAsync(projectId, lead.Id, "Manager");
        await AddAsync(projectId, worker.Id);
        await AddAsync(projectId, client.Id, "Client");

        var members = await TeamAppService.GetMembersAsync();

        var leadRow = members.Items.Single(m => m.UserId == lead.Id);
        leadRow.UserName.ShouldBe("lead");
        leadRow.Name.ShouldBe("LEAD");
        leadRow.Email.ShouldBe("lead@acme.test");
        leadRow.IsActive.ShouldBeTrue();
        leadRow.IsManager.ShouldBeTrue();
        leadRow.IsOwner.ShouldBeFalse();
        leadRow.IsClient.ShouldBeFalse();
        leadRow.ProjectIds.ShouldBe(new[] { projectId });

        var workerRow = members.Items.Single(m => m.UserId == worker.Id);
        workerRow.IsManager.ShouldBeFalse();
        workerRow.IsClient.ShouldBeFalse();
        workerRow.ProjectIds.ShouldBe(new[] { projectId });

        members.Items.Single(m => m.UserId == client.Id).IsClient.ShouldBeTrue();

        leadRow.Role.ShouldBe(WorkspaceRoles.Manager);
        workerRow.Role.ShouldBe(WorkspaceRoles.Member);

        // The caller owns the workspace; the seeded platform administrator is no member of it.
        members.Items.Single(m => m.UserId == CurrentUserId).IsOwner.ShouldBeTrue();
        members.Items.ShouldNotContain(m => m.UserName == "admin");

        // Someone removed from the workspace is no longer listed.
        await TeamAppService.RemoveFromWorkspaceAsync(worker.Id);
        (await TeamAppService.GetMembersAsync()).Items.ShouldNotContain(m => m.UserId == worker.Id);
    }

    [Fact]
    public async Task UpdateMember_Should_Change_Role_And_Rate_And_Sync_The_Manager_Role()
    {
        var projectId = await CreateProjectAsync();
        var user = await CreateUserAsync("promoted");
        await AddAsync(projectId, user.Id, hourlyRate: 10m);

        var promoted = await TeamAppService.UpdateMemberAsync(projectId, user.Id, new UpdateProjectMemberDto { Role = "Manager", HourlyRate = 55m });

        promoted.Role.ShouldBe("Manager");
        promoted.HourlyRate.ShouldBe(55m);
        promoted.UserName.ShouldBe("promoted");
        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeTrue();
        (await TeamAppService.GetProjectMembersAsync(projectId)).Items
            .Single(m => m.UserId == user.Id).HourlyRate.ShouldBe(55m);

        await TeamAppService.UpdateMemberAsync(projectId, user.Id, new UpdateProjectMemberDto { Role = "Worker", HourlyRate = 55m });

        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeFalse(); // no Admin/Manager membership left
    }

    [Fact]
    public async Task UpdateMember_Demotion_Should_Keep_The_Role_While_Another_Project_Still_Needs_It()
    {
        var projectA = await CreateProjectAsync();
        var projectB = await CreateProjectAsync();
        var user = await CreateUserAsync("twohats");
        await AddAsync(projectA, user.Id, "Manager");
        await AddAsync(projectB, user.Id, "Admin");

        await TeamAppService.UpdateMemberAsync(projectA, user.Id, new UpdateProjectMemberDto { Role = "Worker" });

        (await IsInRoleAsync(user.Id, WorkspaceRoles.Manager)).ShouldBeTrue();
    }

    [Fact]
    public async Task UpdateMember_Should_Reject_An_Unknown_Membership()
    {
        var projectId = await CreateProjectAsync();

        await Should.ThrowAsync<EntityNotFoundException>(() =>
            TeamAppService.UpdateMemberAsync(projectId, Guid.NewGuid(), new UpdateProjectMemberDto { Role = "Worker" }));
    }

    [Fact]
    public async Task UpdateMember_Without_Changes_Should_Be_Allowed_Even_On_Your_Own_Membership()
    {
        var projectId = await CreateProjectAsync();

        var unchanged = await TeamAppService.UpdateMemberAsync(projectId, CurrentUserId,
            new UpdateProjectMemberDto { Role = "admin", HourlyRate = 0m });

        unchanged.Role.ShouldBe("Admin");
    }
}
