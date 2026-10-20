using System;
using System.Collections.Generic;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// REQ-WS-07: Workspace Invitation Recipient Decline Lifecycle.
/// Verifies that intended authenticated recipients can decline pending workspace invitations without joining,
/// that wrong-email and unauthenticated callers are rejected, that declined tokens cannot be accepted or used
/// for registration, and that other invitations and accepted memberships remain intact.
/// </summary>
public abstract class InvitationDeclineTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IWorkspaceAppService _workspaceAppService;
    private readonly ITeamAppService _teamAppService;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly ICurrentPrincipalAccessor _principalAccessor;
    private readonly IdentityUserManager _userManager;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected InvitationDeclineTests()
    {
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        _teamAppService = GetRequiredService<ITeamAppService>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _invitationRepository = GetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private IDisposable ActAs(Guid? userId)
    {
        if (userId == null)
        {
            return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity()));
        }

        var claims = new List<Claim> { new(AbpClaimTypes.UserId, userId.Value.ToString()) };
        return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "TestAuth")));
    }

    private async Task<Guid> CreateUserAsync(string email)
    {
        var id = _guidGenerator.Create();
        var user = new IdentityUser(id, email, email);
        (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
        return id;
    }

    private async Task<string> InviteAsync(Guid workspaceId, string email)
    {
        using (_currentWorkspace.Change(workspaceId, WorkspaceRoles.Owner))
        {
            var result = await _teamAppService.InviteMemberAsync(new InviteMemberDto
            {
                Email = email,
                Role = WorkspaceRoles.Member
            });
            result.InvitePath.ShouldNotBeNull().ShouldStartWith("/invite/");
            return result.InvitePath!["/invite/".Length..];
        }
    }

    [Fact]
    public async Task Recipient_Can_Decline_Pending_Invitation_Without_Joining_Workspace()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-1");
        var token = await InviteAsync(ws, "invitee@decline.test");
        var inviteeId = await CreateUserAsync("invitee@decline.test");

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            // Intended recipient declines the invitation
            await _workspaceAppService.DeclineInvitationAsync(token);

            // Token is invalidated and no longer found
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.GetInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        }

        // Invitee has NOT joined the workspace
        (await ServiceProvider.FindWorkspaceMemberAsync(inviteeId)).ShouldBeNull();
    }

    [Fact]
    public async Task Wrong_Email_Authenticated_Caller_Cannot_Decline_Another_Recipients_Invitation()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-2");
        var token = await InviteAsync(ws, "intended@decline.test");
        var inviteeId = await CreateUserAsync("intended@decline.test");
        var strangerId = await CreateUserAsync("stranger@decline.test");

        using (ActAs(strangerId))
        using (_currentWorkspace.Change(null))
        {
            // Stranger attempting to decline another recipient's invitation is rejected
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        // Invitation remains intact and usable for the intended recipient
        using (_currentWorkspace.Change(null))
        {
            var info = await _workspaceAppService.GetInvitationAsync(token);
            info.ShouldNotBeNull();
            info.Email.ShouldBe("intended@decline.test");
            info.Expired.ShouldBeFalse();
        }
    }

    [Fact]
    public async Task Decline_By_Caller_With_Workspace_Membership_Throws_If_Not_Target_Email()
    {
        var wsTarget = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-MemberTarget");
        var wsOther = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-MemberOther");

        // 1. Target workspace has a pending invitation for the intended recipient
        var token = await InviteAsync(wsTarget, "target-invitee@decline.test");
        var targetInviteeId = await CreateUserAsync("target-invitee@decline.test");

        // 2. Caller has an actual active membership in wsTarget (Manager) and in wsOther (Member)
        var memberId = await CreateUserAsync("existing-member@decline.test");
        await ServiceProvider.AddWorkspaceMemberAsync(memberId, WorkspaceRoles.Manager, wsTarget);
        await ServiceProvider.AddWorkspaceMemberAsync(memberId, WorkspaceRoles.Member, wsOther);

        // Verify initial membership state
        var initialTargetMember = await ServiceProvider.FindWorkspaceMemberAsync(memberId, wsTarget);
        initialTargetMember.ShouldNotBeNull();
        initialTargetMember.IsActive.ShouldBeTrue();
        initialTargetMember.Role.ShouldBe(WorkspaceRoles.Manager);

        var initialOtherMember = await ServiceProvider.FindWorkspaceMemberAsync(memberId, wsOther);
        initialOtherMember.ShouldNotBeNull();
        initialOtherMember.IsActive.ShouldBeTrue();

        // 3. Caller acts with active target-workspace ambient context/role and attempts to decline another recipient's invitation
        using (ActAs(memberId))
        using (_currentWorkspace.Change(wsTarget, WorkspaceRoles.Manager))
        {
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        // Also verify when ambient context is unset (endpoint is WorkspaceNotRequired)
        using (ActAs(memberId))
        using (_currentWorkspace.Change(null))
        {
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        // 4. Token remains untouched and completely usable by the real intended recipient
        using (_currentWorkspace.Change(null))
        {
            var info = await _workspaceAppService.GetInvitationAsync(token);
            info.ShouldNotBeNull();
            info.Email.ShouldBe("target-invitee@decline.test");
            info.Expired.ShouldBeFalse();
        }

        using (ActAs(targetInviteeId))
        using (_currentWorkspace.Change(null))
        {
            var acceptResult = await _workspaceAppService.AcceptInvitationAsync(token);
            acceptResult.WorkspaceId.ShouldBe(wsTarget);
        }

        var targetMember = await ServiceProvider.FindWorkspaceMemberAsync(targetInviteeId, wsTarget);
        targetMember.ShouldNotBeNull();
        targetMember.IsActive.ShouldBeTrue();

        // 5. Caller's existing membership in wsTarget remains completely intact and active
        var postTargetMember = await ServiceProvider.FindWorkspaceMemberAsync(memberId, wsTarget);
        postTargetMember.ShouldNotBeNull();
        postTargetMember.IsActive.ShouldBeTrue();
        postTargetMember.Role.ShouldBe(WorkspaceRoles.Manager);

        // 6. Other-workspace membership remains intact
        var postOtherMember = await ServiceProvider.FindWorkspaceMemberAsync(memberId, wsOther);
        postOtherMember.ShouldNotBeNull();
        postOtherMember.IsActive.ShouldBeTrue();
    }

    [Fact]
    public async Task Unauthenticated_Caller_Cannot_Decline_Invitation()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-3");
        var token = await InviteAsync(ws, "unauth@decline.test");

        using (ActAs(null))
        using (_currentWorkspace.Change(null))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _workspaceAppService.DeclineInvitationAsync(token));
        }

        // Token remains untouched and usable
        using (_currentWorkspace.Change(null))
        {
            var info = await _workspaceAppService.GetInvitationAsync(token);
            info.ShouldNotBeNull();
            info.Expired.ShouldBeFalse();
        }
    }

    [Fact]
    public async Task Declined_Token_Cannot_Be_Accepted_By_Existing_Account()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-4");
        var token = await InviteAsync(ws, "existing@decline.test");
        var inviteeId = await CreateUserAsync("existing@decline.test");

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            await _workspaceAppService.DeclineInvitationAsync(token);

            // Attempting to accept the declined token fails with InvitationNotFound
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.AcceptInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(inviteeId)).ShouldBeNull();
    }

    [Fact]
    public async Task Declined_Token_Cannot_Be_Used_For_New_Registration()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-5");
        var token = await InviteAsync(ws, "regdecline@decline.test");
        var inviteeId = await CreateUserAsync("regdecline@decline.test");

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            await _workspaceAppService.DeclineInvitationAsync(token);
        }

        // Attempting to register with the declined token fails with InvitationNotFound
        var regDto = new RegisterWorkspaceDto
        {
            Email = "freshuser@decline.test",
            Password = "1q2w3E*fresh",
            InviteToken = token
        };

        using (_currentWorkspace.Change(null))
        {
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.RegisterAsync(regDto));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        }
    }

    [Fact]
    public async Task Declining_One_Workspace_Invitation_Does_Not_Affect_Other_Workspace_Invitations()
    {
        var wsA = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-B");

        var tokenA = await InviteAsync(wsA, "multiinvite@decline.test");
        var tokenB = await InviteAsync(wsB, "multiinvite@decline.test");
        var inviteeId = await CreateUserAsync("multiinvite@decline.test");

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            // Decline workspace A invitation
            await _workspaceAppService.DeclineInvitationAsync(tokenA);

            // Token A is gone
            var exA = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.GetInvitationAsync(tokenA));
            exA.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);

            // Token B is still valid and can be accepted
            var acceptedB = await _workspaceAppService.AcceptInvitationAsync(tokenB);
            acceptedB.WorkspaceId.ShouldBe(wsB);
        }

        // Invitee belongs to WS-B only, NOT WS-A
        var memberB = await ServiceProvider.FindWorkspaceMemberAsync(inviteeId, wsB);
        memberB.ShouldNotBeNull();
        memberB.IsActive.ShouldBeTrue();

        var memberA = await ServiceProvider.FindWorkspaceMemberAsync(inviteeId, wsA);
        memberA.ShouldBeNull();
    }

    [Fact]
    public async Task Already_Accepted_Invitation_Cannot_Be_Declined_And_Existing_Membership_Preserved()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-Accepted");
        var token = await InviteAsync(ws, "accepted@decline.test");
        var inviteeId = await CreateUserAsync("accepted@decline.test");

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            // First accept the invitation
            var accepted = await _workspaceAppService.AcceptInvitationAsync(token);
            accepted.WorkspaceId.ShouldBe(ws);

            // Attempting to decline the already accepted invitation fails
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync(token));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationExpired);
        }

        // Membership in the workspace remains fully intact and active
        var member = await ServiceProvider.FindWorkspaceMemberAsync(inviteeId, ws);
        member.ShouldNotBeNull();
        member.IsActive.ShouldBeTrue();
    }

    [Fact]
    public async Task Expired_Invitation_Cannot_Be_Declined()
    {
        var ws = await ServiceProvider.CreateWorkspaceAsync("Decline-WS-Expired");
        var inviteeId = await CreateUserAsync("expired@decline.test");

        var rawToken = WorkspaceInvitation.NewToken();
        var expiredInvitation = new WorkspaceInvitation(
            _guidGenerator.Create(),
            ws,
            "expired@decline.test",
            WorkspaceRoles.Member,
            projectId: null,
            projectRole: null,
            hourlyRate: null,
            WorkspaceInvitation.HashToken(rawToken),
            DateTime.UtcNow.AddDays(-1)); // expired yesterday

        await WithUnitOfWorkAsync(() => _invitationRepository.InsertAsync(expiredInvitation));

        using (ActAs(inviteeId))
        using (_currentWorkspace.Change(null))
        {
            var ex = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync(rawToken));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvitationExpired);
        }
    }

    [Fact]
    public async Task Nonexistent_Or_Malformed_Token_Cannot_Be_Declined()
    {
        var userId = await CreateUserAsync("anyone@decline.test");

        using (ActAs(userId))
        using (_currentWorkspace.Change(null))
        {
            var exNotFound = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync("completely-unknown-token"));
            exNotFound.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);

            var exEmpty = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.DeclineInvitationAsync("   "));
            exEmpty.Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        }
    }
}
