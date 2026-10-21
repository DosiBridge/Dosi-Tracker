using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>
/// REQ-WS-07 / REQ-ISO-05: Hosted HTTP Workspace Invitation Recipient Decline Lifecycle & Security Acceptance.
/// Uses the hosted ASP.NET Core TestServer MVC pipeline, disposable project-local SQLite,
/// real production WorkspaceAppService, InvitationsController, WorkspaceResolutionMiddleware,
/// WorkspaceRequiredFilter, and production exception-to-status mappings.
/// </summary>
public class WorkspaceInvitationDeclineHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb31");
    private static readonly Guid ManagerA = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb32");
    private static readonly Guid MemberA = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb33");
    private static readonly Guid InviteeUser = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb34");
    private static readonly Guid StrangerUser = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb35");
    private static readonly Guid DualUser = Guid.Parse("5f701e62-0953-4dd3-910b-dc6cc93ccb36");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IWorkspaceAppService _workspaceAppService;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceInvitationDeclineHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _invitationRepository = GetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
    }

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var db = ServiceProvider.GetRequiredService<TrackerDbContext>();
        var conn = db.Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
        {
            options.Configure(ctx =>
            {
                ctx.DbContextOptions.UseSqlite(conn);
            });
        });
        services.AddAlwaysDisableUnitOfWorkTransaction();
        services.AddControllers().AddApplicationPart(typeof(WorkspacesController).Assembly);
    }

    private async Task EnsureUserAsync(Guid id, string userName, string email)
    {
        if (await _userManager.FindByIdAsync(id.ToString()) == null)
        {
            var user = new IdentityUser(id, userName, email);
            (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
        }
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA)> SeedFixtureAsync()
    {
        await EnsureUserAsync(OwnerA, "ownerA", "ownerA@acme.test");
        await EnsureUserAsync(ManagerA, "managerA", "managerA@acme.test");
        await EnsureUserAsync(MemberA, "memberA", "memberA@acme.test");
        await EnsureUserAsync(InviteeUser, "inviteeUser", "invitee@acme.test");
        await EnsureUserAsync(StrangerUser, "strangerUser", "stranger@acme.test");
        await EnsureUserAsync(DualUser, "dualUser", "dual@acme.test");

        var wsA = await ServiceProvider.CreateWorkspaceAsync("DECLINE-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("DECLINE-WS-B");

        // Workspace A memberships
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerA, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(ManagerA, WorkspaceRoles.Manager, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(MemberA, WorkspaceRoles.Member, wsA);

        // Workspace B memberships
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsB);

        // Concrete project sentinel in Workspace A
        var projA = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _projectRepository.InsertAsync(new Project(projA, wsA, "Sentinel Project A"), autoSave: true);
            }
        });

        return (wsA, wsB, projA);
    }

    private async Task<(string RawToken, WorkspaceInvitation Invitation)> SeedInvitationAsync(
        Guid workspaceId,
        string email,
        string role = WorkspaceRoles.Member,
        DateTime? expiresAt = null,
        Guid? projectId = null)
    {
        var rawToken = WorkspaceInvitation.NewToken();
        var id = _guidGenerator.Create();
        var invitation = new WorkspaceInvitation(
            id: id,
            workspaceId: workspaceId,
            email: email,
            role: role,
            projectId: projectId,
            projectRole: null,
            hourlyRate: null,
            tokenHash: WorkspaceInvitation.HashToken(rawToken),
            expiresAt: expiresAt ?? DateTime.UtcNow.AddDays(7));

        await WithUnitOfWorkAsync(async () =>
        {
            await _invitationRepository.InsertAsync(invitation, autoSave: true);
        });

        return (rawToken, invitation);
    }

    [Fact]
    public async Task Real_Hosted_Route_Recipient_Can_Decline_Own_Invitation_Without_Workspace_Header()
    {
        var (wsA, _, projA) = await SeedFixtureAsync();
        var (rawToken, invitation) = await SeedInvitationAsync(wsA, "invitee@acme.test", projectId: projA);
        var client = CreateHttpClient();

        // Recipient is not yet a member of Workspace A, sends decline with NO X-Workspace-Id header
        var declineReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: InviteeUser, workspaceId: null);
        var declineResp = await client.SendAsync(declineReq);

        declineResp.StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // 1. Invitation is deleted from database
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            var inv = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash);
            inv.ShouldBeNull();
        });

        // 2. Invitee has NOT joined Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == InviteeUser);
            member.ShouldBeNull();
        });

        // 3. Sentinels in Workspace A remain intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var project = await _projectRepository.FindAsync(projA);
                project.ShouldNotBeNull();
                project.Title.ShouldBe("Sentinel Project A");

                var ownerMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == OwnerA);
                ownerMember.ShouldNotBeNull();
                ownerMember.Role.ShouldBe(WorkspaceRoles.Owner);
            }
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Recipient_With_Existing_Membership_In_Other_Workspace_Can_Decline_With_Or_Without_Workspace_Header()
    {
        var (wsA, wsB, projA) = await SeedFixtureAsync();
        var client = CreateHttpClient();

        // Invitation 1: For DualUser to join Workspace A
        var (tokenA, _) = await SeedInvitationAsync(wsA, "dual@acme.test", projectId: projA);

        // Subcase A: DualUser declines Workspace A invitation while supplying valid X-Workspace-Id for Workspace B (where they are active member)
        var reqWithWsB = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA}/decline", userId: DualUser, workspaceId: wsB);
        var respWithWsB = await client.SendAsync(reqWithWsB);
        respWithWsB.StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // Proves tokenA is deleted
        await WithUnitOfWorkAsync(async () =>
        {
            var hashA = WorkspaceInvitation.HashToken(tokenA);
            (await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hashA)).ShouldBeNull();
        });

        // Proves DualUser's membership in Workspace B is completely unaffected and still active
        await WithUnitOfWorkAsync(async () =>
        {
            var memberB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == DualUser);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
            memberB.Role.ShouldBe(WorkspaceRoles.Member);

            // Did NOT join Workspace A
            var memberA = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == DualUser);
            memberA.ShouldBeNull();
        });

        // Subcase B: DualUser receives another invitation to Workspace A and declines with NO workspace header
        var (tokenA2, _) = await SeedInvitationAsync(wsA, "dual@acme.test");
        var reqWithoutWs = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA2}/decline", userId: DualUser, workspaceId: null);
        var respWithoutWs = await client.SendAsync(reqWithoutWs);
        respWithoutWs.StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // Membership in Workspace B still intact
        await WithUnitOfWorkAsync(async () =>
        {
            var memberB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == DualUser);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Unauthenticated_Decline_Returns_401Unauthorized_And_Preserves_Invitation()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        var (rawToken, invitation) = await SeedInvitationAsync(wsA, "invitee@acme.test");
        var client = CreateHttpClient();

        // Caller has NO authorization header
        var req = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: null, workspaceId: null);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("Volo.Abp.Authorization:010001");

        // Invitation remains intact in database
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            var inv = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash);
            inv.ShouldNotBeNull();
            inv.Id.ShouldBe(invitation.Id);
            inv.Email.ShouldBe("invitee@acme.test");
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Wrong_Email_Stranger_Returns_403Forbidden_And_Preserves_Invitation()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        var (rawToken, invitation) = await SeedInvitationAsync(wsA, "invitee@acme.test");
        var client = CreateHttpClient();

        // StrangerUser (stranger@acme.test) attempts to decline invitee@acme.test's invitation
        var req = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: StrangerUser, workspaceId: null);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.InvitationEmailMismatch);

        // Invitation is preserved in database
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            var inv = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash);
            inv.ShouldNotBeNull();
            inv.Id.ShouldBe(invitation.Id);
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Existing_Member_Of_Target_Workspace_Cannot_Decline_Another_Recipients_Invitation()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        var (rawToken, invitation) = await SeedInvitationAsync(wsA, "invitee@acme.test");
        var client = CreateHttpClient();

        // 1. ManagerA (managerA@acme.test, an active member of Workspace A) attempts to decline invitee's token with Workspace A header
        var reqWithWs = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: ManagerA, workspaceId: wsA);
        var respWithWs = await client.SendAsync(reqWithWs);

        respWithWs.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await respWithWs.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationEmailMismatch);

        // 2. ManagerA attempts to decline without workspace header
        var reqNoWs = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: ManagerA, workspaceId: null);
        var respNoWs = await client.SendAsync(reqNoWs);

        respNoWs.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await respNoWs.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationEmailMismatch);

        // Invitation is preserved
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            (await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash)).ShouldNotBeNull();

            // ManagerA's membership is preserved
            var mgr = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == ManagerA);
            mgr.ShouldNotBeNull();
            mgr.Role.ShouldBe(WorkspaceRoles.Manager);
            mgr.IsActive.ShouldBeTrue();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Foreign_Workspace_Header_Where_Caller_Not_Member_Returns_403NotWorkspaceMember_And_Preserves_Invitation()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        var (rawToken, invitation) = await SeedInvitationAsync(wsA, "invitee@acme.test");
        var client = CreateHttpClient();

        // Foreign workspace where InviteeUser is NOT a member
        var foreignWorkspaceId = _guidGenerator.Create();
        await ServiceProvider.CreateWorkspaceAsync("FOREIGN-WS");

        var req = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: InviteeUser, workspaceId: foreignWorkspaceId);
        var resp = await client.SendAsync(req);

        // Intercepted by WorkspaceResolutionMiddleware
        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Invitation remains intact in DB
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            (await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash)).ShouldNotBeNull();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Declined_Invitation_Cannot_Be_Accepted_On_Accept_Endpoint()
    {
        var (wsA, _, projA) = await SeedFixtureAsync();
        var (rawToken, _) = await SeedInvitationAsync(wsA, "invitee@acme.test", projectId: projA);
        var client = CreateHttpClient();

        // 1. Legitimate decline
        var declineReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: InviteeUser, workspaceId: null);
        var declineResp = await client.SendAsync(declineReq);
        declineResp.StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // 2. Attempt to accept the declined token on the real accept route
        var acceptReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/accept", userId: InviteeUser, workspaceId: null);
        var acceptResp = await client.SendAsync(acceptReq);

        acceptResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        var body = await acceptResp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.InvitationNotFound);

        // 3. Invitee did NOT join Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == InviteeUser);
            member.ShouldBeNull();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Declining_Workspace_A_Invitation_Preserves_Separate_Workspace_B_Invitation_For_Same_Account()
    {
        var (wsA, wsB, _) = await SeedFixtureAsync();
        var client = CreateHttpClient();

        // User multiInvitee has pending invitations to both Workspace A (as Member) and Workspace B (as Manager)
        var multiUserId = _guidGenerator.Create();
        await EnsureUserAsync(multiUserId, "multiUser", "multi@acme.test");

        var (tokenA, _) = await SeedInvitationAsync(wsA, "multi@acme.test", role: WorkspaceRoles.Member);
        var (tokenB, _) = await SeedInvitationAsync(wsB, "multi@acme.test", role: WorkspaceRoles.Manager);

        // 1. Decline Workspace A invitation
        var declineReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA}/decline", userId: multiUserId, workspaceId: null);
        var declineResp = await client.SendAsync(declineReq);
        declineResp.StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // Proves tokenA is deleted
        await WithUnitOfWorkAsync(async () =>
        {
            var hashA = WorkspaceInvitation.HashToken(tokenA);
            (await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hashA)).ShouldBeNull();

            // tokenB is still present and valid
            var hashB = WorkspaceInvitation.HashToken(tokenB);
            var invB = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hashB);
            invB.ShouldNotBeNull();
            invB.Role.ShouldBe(WorkspaceRoles.Manager);
        });

        // 2. Accept Workspace B invitation on the real accept route
        var acceptReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenB}/accept", userId: multiUserId, workspaceId: null);
        var acceptResp = await client.SendAsync(acceptReq);
        acceptResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. User joined Workspace B as Manager, and did NOT join Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var memberB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == multiUserId);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
            memberB.Role.ShouldBe(WorkspaceRoles.Manager);

            var memberA = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == multiUserId);
            memberA.ShouldBeNull();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Already_Accepted_Invitation_Cannot_Be_Declined_Returns_410Gone_And_Preserves_Membership()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        var (rawToken, _) = await SeedInvitationAsync(wsA, "invitee@acme.test");
        var client = CreateHttpClient();

        // 1. Accept invitation
        var acceptReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/accept", userId: InviteeUser, workspaceId: null);
        var acceptResp = await client.SendAsync(acceptReq);
        acceptResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // Verify user joined
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == InviteeUser);
            member.ShouldNotBeNull();
            member.IsActive.ShouldBeTrue();
        });

        // 2. Attempt to decline the already accepted invitation
        var declineReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: InviteeUser, workspaceId: null);
        var declineResp = await client.SendAsync(declineReq);

        // Maps to 410 Gone (InvitationExpired)
        declineResp.StatusCode.ShouldBe(HttpStatusCode.Gone);
        var body = await declineResp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.InvitationExpired);

        // Membership remains fully intact and active
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == InviteeUser);
            member.ShouldNotBeNull();
            member.IsActive.ShouldBeTrue();
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Expired_Invitation_Cannot_Be_Declined_Returns_410Gone_And_Preserves_Database_Row()
    {
        var (wsA, _, _) = await SeedFixtureAsync();
        // Seed expired invitation (expired 2 days ago)
        var (rawToken, expiredInv) = await SeedInvitationAsync(wsA, "invitee@acme.test", expiresAt: DateTime.UtcNow.AddDays(-2));
        var client = CreateHttpClient();

        var declineReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/decline", userId: InviteeUser, workspaceId: null);
        var declineResp = await client.SendAsync(declineReq);

        // Maps to 410 Gone (InvitationExpired)
        declineResp.StatusCode.ShouldBe(HttpStatusCode.Gone);
        var body = await declineResp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.InvitationExpired);

        // Database row is preserved (not deleted)
        await WithUnitOfWorkAsync(async () =>
        {
            var hash = WorkspaceInvitation.HashToken(rawToken);
            var inv = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash);
            inv.ShouldNotBeNull();
            inv.Id.ShouldBe(expiredInv.Id);
        });
    }

    [Fact]
    public async Task Real_Hosted_Route_Nonexistent_Or_Malformed_Token_Returns_404NotFound()
    {
        await SeedFixtureAsync();
        var client = CreateHttpClient();

        // 1. Unknown token
        var unknownReq = CreateRequest(HttpMethod.Post, "/api/app/invitations/completely-unknown-token/decline", userId: InviteeUser, workspaceId: null);
        var unknownResp = await client.SendAsync(unknownReq);
        unknownResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await unknownResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationNotFound);

        // 2. Whitespace token fails route parameter / model validation -> 400 Bad Request
        var wsReq = CreateRequest(HttpMethod.Post, "/api/app/invitations/%20%20%20/decline", userId: InviteeUser, workspaceId: null);
        var wsResp = await client.SendAsync(wsReq);
        wsResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task Real_Hosted_Route_Preserves_Nonempty_Sentinels_Across_Denied_And_Declined_Requests()
    {
        var (wsA, wsB, projA) = await SeedFixtureAsync();
        var (tokenA, _) = await SeedInvitationAsync(wsA, "invitee@acme.test", projectId: projA);
        var (tokenB, _) = await SeedInvitationAsync(wsB, "other_invitee@acme.test");
        var client = CreateHttpClient();

        // 1. Unauthenticated attempt on tokenA -> 401
        var unauthReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA}/decline", userId: null, workspaceId: null);
        (await client.SendAsync(unauthReq)).StatusCode.ShouldBe(HttpStatusCode.Unauthorized);

        // 2. Wrong-email attempt on tokenA -> 403
        var wrongEmailReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA}/decline", userId: StrangerUser, workspaceId: null);
        (await client.SendAsync(wrongEmailReq)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 3. Legitimate decline of tokenA -> 204
        var legitReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{tokenA}/decline", userId: InviteeUser, workspaceId: null);
        (await client.SendAsync(legitReq)).StatusCode.ShouldBe(HttpStatusCode.NoContent);

        // 4. Thorough DB sentinel inspection:
        await WithUnitOfWorkAsync(async () =>
        {
            // tokenA is deleted
            var hashA = WorkspaceInvitation.HashToken(tokenA);
            (await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hashA)).ShouldBeNull();

            // tokenB in Workspace B is completely untouched
            var hashB = WorkspaceInvitation.HashToken(tokenB);
            var invB = await _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hashB);
            invB.ShouldNotBeNull();
            invB.WorkspaceId.ShouldBe(wsB);
            invB.Email.ShouldBe("other_invitee@acme.test");

            // Sentinel project in Workspace A remains intact
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var proj = await _projectRepository.FindAsync(projA);
                proj.ShouldNotBeNull();
                proj.WorkspaceId.ShouldBe(wsA);

                // Memberships in Workspace A (Owner, Manager, Member) remain intact
                var membersA = await _memberRepository.GetListAsync(m => m.WorkspaceId == wsA);
                membersA.Count.ShouldBe(3);
                membersA.ShouldContain(m => m.UserId == OwnerA && m.Role == WorkspaceRoles.Owner);
                membersA.ShouldContain(m => m.UserId == ManagerA && m.Role == WorkspaceRoles.Manager);
                membersA.ShouldContain(m => m.UserId == MemberA && m.Role == WorkspaceRoles.Member);

                // Memberships in Workspace B remain intact
                var membersB = await _memberRepository.GetListAsync(m => m.WorkspaceId == wsB);
                membersB.Count.ShouldBe(1);
                membersB.ShouldContain(m => m.UserId == DualUser && m.Role == WorkspaceRoles.Member);
            }
        });
    }
}
