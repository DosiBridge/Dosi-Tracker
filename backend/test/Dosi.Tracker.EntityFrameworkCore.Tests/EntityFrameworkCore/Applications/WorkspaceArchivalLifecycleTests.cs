using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
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
/// REQ-WS-08: Reversible owner-controlled workspace archival and restoration integration suite.
/// Uses the hosted ASP.NET Core TestServer MVC pipeline, disposable project-local SQLite,
/// real WorkspaceManager domain logic, real production WorkspaceAppService, WorkspacesController,
/// WorkspaceResolutionMiddleware, and WorkspaceRequiredFilter.
/// </summary>
public class WorkspaceArchivalLifecycleTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb11");
    private static readonly Guid ManagerA = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb12");
    private static readonly Guid MemberA = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb13");
    private static readonly Guid InactiveOwnerA = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb14");
    private static readonly Guid UserB = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb15");
    private static readonly Guid DualUser = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb16");
    private static readonly Guid InviteeUser = Guid.Parse("4f701e62-0953-4dd3-910b-dc6cc93ccb17");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IWorkspaceAppService _workspaceAppService;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceArchivalLifecycleTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
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

    private async Task<(Guid WorkspaceA, Guid WorkspaceB)> SeedArchivalFixtureAsync()
    {
        async Task EnsureUserAsync(Guid id, string userName, string email)
        {
            if (await _userManager.FindByIdAsync(id.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(id, userName, email));
            }
        }

        await EnsureUserAsync(OwnerA, "ownerA", "ownerA@acme.test");
        await EnsureUserAsync(ManagerA, "managerA", "managerA@acme.test");
        await EnsureUserAsync(MemberA, "memberA", "memberA@acme.test");
        await EnsureUserAsync(InactiveOwnerA, "inactiveOwnerA", "inactiveOwnerA@acme.test");
        await EnsureUserAsync(UserB, "userB", "userB@acme.test");
        await EnsureUserAsync(DualUser, "dualUser", "dualUser@acme.test");
        await EnsureUserAsync(InviteeUser, "inviteeUser", "inviteeUser@acme.test");

        var wsA = await ServiceProvider.CreateWorkspaceAsync("ARCHIVE-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("ARCHIVE-WS-B");

        // Workspace A memberships
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerA, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(ManagerA, WorkspaceRoles.Manager, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(MemberA, WorkspaceRoles.Member, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsA);

        // Inactive former owner in Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var inactive = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveOwnerA, WorkspaceRoles.Owner, DateTime.UtcNow);
            inactive.Remove();
            await _memberRepository.InsertAsync(inactive, autoSave: true);
        });

        // Workspace B memberships
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Owner, wsB);

        // Seed sentinel project in Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var projectA = new Project(_guidGenerator.Create(), wsA, "WS-A-Sentinel-Project", color: "#112233");
                await _projectRepository.InsertAsync(projectA, autoSave: true);
            }
        });

        // Seed sentinel project in Workspace B
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var projectB = new Project(_guidGenerator.Create(), wsB, "WS-B-Sentinel-Project", color: "#445566");
                await _projectRepository.InsertAsync(projectB, autoSave: true);
            }
        });

        return (wsA, wsB);
    }

    [Fact]
    public async Task Owner_Can_Archive_Workspace_Successfully_And_Listing_Visibly_Reflects_Archived_State()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // Owner calls POST /api/app/workspaces/{id}/archive
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        var archiveResp = await client.SendAsync(archiveReq);

        archiveResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var body = await archiveResp.Content.ReadAsStringAsync();
        using var doc = JsonDocument.Parse(body);
        doc.RootElement.GetProperty("isArchived").GetBoolean().ShouldBeTrue();
        doc.RootElement.GetProperty("id").GetString().ShouldBe(wsA.ToString());

        // Verify persisted state in database
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeTrue();
            ws.ArchivedBy.ShouldBe(OwnerA);
            ws.ArchivedTime.ShouldNotBeNull();
        });

        // Verify GET /api/app/workspaces/mine visibly represents archived state
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: OwnerA, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var mineBody = await mineResp.Content.ReadAsStringAsync();
        using var mineDoc = JsonDocument.Parse(mineBody);
        var myWs = mineDoc.RootElement.EnumerateArray().FirstOrDefault(e => e.GetProperty("id").GetString() == wsA.ToString());
        myWs.ValueKind.ShouldNotBe(JsonValueKind.Undefined);
        myWs.GetProperty("isArchived").GetBoolean().ShouldBeTrue();
        myWs.GetProperty("archivedTime").GetString().ShouldNotBeNull();
    }

    [Fact]
    public async Task Outsider_Manager_Member_And_Inactive_Former_Owner_Are_Denied_Archive_With_Unchanged_State()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // 1. Outsider UserB (not in Workspace A) attempts archive
        var outsiderReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: UserB, workspaceId: null);
        var outsiderResp = await client.SendAsync(outsiderReq);
        outsiderResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 2. ManagerA in Workspace A attempts archive
        var managerReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: ManagerA, workspaceId: null);
        var managerResp = await client.SendAsync(managerReq);
        managerResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 3. MemberA in Workspace A attempts archive
        var memberReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: MemberA, workspaceId: null);
        var memberResp = await client.SendAsync(memberReq);
        memberResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 4. Inactive former owner InactiveOwnerA attempts archive
        var inactiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: InactiveOwnerA, workspaceId: null);
        var inactiveResp = await client.SendAsync(inactiveReq);
        inactiveResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Assert persisted state remains completely unarchived
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeFalse();
            ws.ArchivedBy.ShouldBeNull();
            ws.ArchivedTime.ShouldBeNull();
        });
    }

    [Fact]
    public async Task Archived_Workspace_Normal_Requests_Are_Denied_With_403_WorkspaceArchived()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // Owner archives Workspace A
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 1. Owner attempts ordinary workspace request: GET /api/app/project with X-Workspace-Id: wsA
        var ownerGetReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: OwnerA, workspaceId: wsA);
        var ownerGetResp = await client.SendAsync(ownerGetReq);
        ownerGetResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var ownerGetBody = await ownerGetResp.Content.ReadAsStringAsync();
        ownerGetBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceArchived);
        ownerGetBody.ShouldContain("This workspace is archived.");

        // 2. Manager attempts ordinary mutation: POST /api/app/project with X-Workspace-Id: wsA
        var managerPostReq = CreateRequest(HttpMethod.Post, "/api/app/project", userId: ManagerA, workspaceId: wsA, new
        {
            Title = "Should Not Be Created",
            Color = "#990000"
        });
        var managerPostResp = await client.SendAsync(managerPostReq);
        managerPostResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var managerPostBody = await managerPostResp.Content.ReadAsStringAsync();
        managerPostBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceArchived);

        // Verify no new project was inserted in DB
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var count = await _projectRepository.CountAsync(p => p.WorkspaceId == wsA && p.Title == "Should Not Be Created");
                count.ShouldBe(0);
            }
        });
    }

    [Fact]
    public async Task Archiving_Workspace_A_Preserves_Legitimate_Access_To_Unrelated_Workspace_B()
    {
        var (wsA, wsB) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // Archive Workspace A
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // DualUser (member of A, owner of B) sends normal request to Workspace B
        var bGetReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: DualUser, workspaceId: wsB);
        var bGetResp = await client.SendAsync(bGetReq);
        bGetResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var bGetBody = await bGetResp.Content.ReadAsStringAsync();
        bGetBody.ShouldContain("WS-B-Sentinel-Project");

        // DualUser creates a project in Workspace B
        var bPostReq = CreateRequest(HttpMethod.Post, "/api/app/project", userId: DualUser, workspaceId: wsB, new
        {
            Title = "WS-B-Active-Project",
            Color = "#00AA00"
        });
        var bPostResp = await client.SendAsync(bPostReq);
        bPostResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // Direct DB verification: project created in WS-B
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var created = await _projectRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsB && p.Title == "WS-B-Active-Project");
                created.ShouldNotBeNull();
            }
        });
    }

    [Fact]
    public async Task Owner_Can_Restore_Archived_Workspace_Reenabling_Normal_Access()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // 1. Archive WS-A
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Normal access is denied while archived
        var getArchivedReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: OwnerA, workspaceId: wsA);
        (await client.SendAsync(getArchivedReq)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 3. Owner restores WS-A (even with remembered header or without header)
        var restoreReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: OwnerA, workspaceId: wsA);
        var restoreResp = await client.SendAsync(restoreReq);
        restoreResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var restoreBody = await restoreResp.Content.ReadAsStringAsync();
        using var doc = JsonDocument.Parse(restoreBody);
        doc.RootElement.GetProperty("isArchived").GetBoolean().ShouldBeFalse();
        doc.RootElement.GetProperty("archivedTime").ValueKind.ShouldBe(JsonValueKind.Null);
        doc.RootElement.GetProperty("archivedBy").ValueKind.ShouldBe(JsonValueKind.Null);

        // 4. Verify DB entity is active
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeFalse();
            ws.ArchivedBy.ShouldBeNull();
            ws.ArchivedTime.ShouldBeNull();
        });

        // 5. Normal access is immediately re-enabled (200 OK)
        var getRestoredReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: OwnerA, workspaceId: wsA);
        var getRestoredResp = await client.SendAsync(getRestoredReq);
        getRestoredResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var getRestoredBody = await getRestoredResp.Content.ReadAsStringAsync();
        getRestoredBody.ShouldContain("WS-A-Sentinel-Project");
    }

    [Fact]
    public async Task Outsider_Manager_Member_And_Inactive_Former_Owner_Are_Denied_Restore()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // Archive WS-A first
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 1. Outsider UserB attempts restore
        var outsiderRestore = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: UserB, workspaceId: null);
        (await client.SendAsync(outsiderRestore)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 2. ManagerA attempts restore
        var managerRestore = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: ManagerA, workspaceId: null);
        (await client.SendAsync(managerRestore)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 3. MemberA attempts restore
        var memberRestore = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: MemberA, workspaceId: null);
        (await client.SendAsync(memberRestore)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 4. Inactive former owner attempts restore
        var inactiveRestore = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: InactiveOwnerA, workspaceId: null);
        (await client.SendAsync(inactiveRestore)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Verify Workspace A remains strictly archived in DB
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeTrue();
            ws.ArchivedBy.ShouldBe(OwnerA);
        });
    }

    [Fact]
    public async Task Archive_And_Restore_Are_Idempotent_Preserving_Audit_State()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // 1. First archive
        var req1 = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        var resp1 = await client.SendAsync(req1);
        resp1.StatusCode.ShouldBe(HttpStatusCode.OK);

        DateTime? firstArchivedTime = null;
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeTrue();
            firstArchivedTime = ws.ArchivedTime;
        });

        // 2. Repeat archive (Idempotency): returns 200 OK without error
        var req2 = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        var resp2 = await client.SendAsync(req2);
        resp2.StatusCode.ShouldBe(HttpStatusCode.OK);

        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeTrue();
            ws.ArchivedTime.ShouldBe(firstArchivedTime); // Unmodified
        });

        // 3. First restore
        var restoreReq1 = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: OwnerA, workspaceId: null);
        var restoreResp1 = await client.SendAsync(restoreReq1);
        restoreResp1.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 4. Repeat restore (Idempotency): returns 200 OK without error
        var restoreReq2 = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: OwnerA, workspaceId: null);
        var restoreResp2 = await client.SendAsync(restoreReq2);
        restoreResp2.StatusCode.ShouldBe(HttpStatusCode.OK);

        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.IsArchived.ShouldBeFalse();
            ws.ArchivedTime.ShouldBeNull();
            ws.ArchivedBy.ShouldBeNull();
        });
    }

    [Fact]
    public async Task Archival_Preserves_All_Data_Memberships_Subscriptions_And_Sentinels_Without_Deletion_Or_Cancellation()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // Seed subscription in Workspace A
        Guid subscriptionId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            var plan = await _planRepository.FirstAsync(p => p.Name == Plan.FreePlanName);
            var sub = new Subscription(subscriptionId, wsA, plan.Id)
            {
                Status = Subscription.ActiveStatus
            };
            await _subscriptionRepository.InsertAsync(sub, autoSave: true);
        });

        // Count pre-archival rows
        int preMemberCount = 0;
        int preProjectCount = 0;
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                preMemberCount = await _memberRepository.CountAsync(m => m.WorkspaceId == wsA);
                preProjectCount = await _projectRepository.CountAsync(p => p.WorkspaceId == wsA);
            }
        });
        preMemberCount.ShouldBeGreaterThan(0);
        preProjectCount.ShouldBeGreaterThan(0);

        // Archive Workspace A
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // Verify rows are NOT deleted or cancelled while archived
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var postMemberCount = await _memberRepository.CountAsync(m => m.WorkspaceId == wsA);
                var postProjectCount = await _projectRepository.CountAsync(p => p.WorkspaceId == wsA);
                var sub = await _subscriptionRepository.GetAsync(subscriptionId);

                postMemberCount.ShouldBe(preMemberCount);
                postProjectCount.ShouldBe(preProjectCount);
                sub.Status.ShouldBe(Subscription.ActiveStatus); // NOT cancelled
            }
        });

        // Restore Workspace A
        var restoreReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/restore", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(restoreReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // Verify sentinel data remains fully intact and accessible via HTTP
        var getReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: OwnerA, workspaceId: wsA);
        var getResp = await client.SendAsync(getReq);
        getResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        (await getResp.Content.ReadAsStringAsync()).ShouldContain("WS-A-Sentinel-Project");
    }

    [Fact]
    public async Task Invitation_Acceptance_And_Registration_Into_Archived_Workspace_Are_Rejected_Before_Mutation()
    {
        var (wsA, _) = await SeedArchivalFixtureAsync();
        var client = CreateHttpClient();

        // 1. Seed a valid pending invitation token for Workspace A
        var rawToken = WorkspaceInvitation.NewToken();
        var tokenHash = WorkspaceInvitation.HashToken(rawToken);
        var inviteEmail = "pending_invitee@acme.test";
        Guid inviteId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            var invite = new WorkspaceInvitation(
                inviteId,
                wsA,
                inviteEmail,
                WorkspaceRoles.Member,
                projectId: null,
                projectRole: null,
                hourlyRate: null,
                tokenHash,
                DateTime.UtcNow.AddDays(7));
            await _invitationRepository.InsertAsync(invite, autoSave: true);
        });

        // 2. Archive Workspace A
        var archiveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/archive", userId: OwnerA, workspaceId: null);
        (await client.SendAsync(archiveReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. User with matching email attempts to accept invitation into archived workspace
        // Ensure user exists
        Guid inviteeUserId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            await _userManager.CreateAsync(new IdentityUser(inviteeUserId, "pending_invitee", inviteEmail));
        });

        var acceptReq = CreateRequest(HttpMethod.Post, $"/api/app/invitations/{rawToken}/accept", userId: inviteeUserId, workspaceId: null);
        var acceptResp = await client.SendAsync(acceptReq);
        acceptResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var acceptBody = await acceptResp.Content.ReadAsStringAsync();
        acceptBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceArchived);

        // Verify DB: invitation is NOT marked accepted, and NO membership was created
        await WithUnitOfWorkAsync(async () =>
        {
            var inv = await _invitationRepository.GetAsync(inviteId);
            inv.AcceptedUserId.ShouldBeNull();
            inv.AcceptedAt.ShouldBeNull();

            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == inviteeUserId);
            member.ShouldBeNull();
        });

        // 4. New anonymous registration using an invite token into an archived workspace is also rejected before account/membership creation
        var rawRegisterToken = WorkspaceInvitation.NewToken();
        var registerTokenHash = WorkspaceInvitation.HashToken(rawRegisterToken);
        var registerEmail = "brand_new_signup@acme.test";
        Guid registerInviteId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            var invite = new WorkspaceInvitation(
                registerInviteId,
                wsA,
                registerEmail,
                WorkspaceRoles.Member,
                projectId: null,
                projectRole: null,
                hourlyRate: null,
                registerTokenHash,
                DateTime.UtcNow.AddDays(7));
            await _invitationRepository.InsertAsync(invite, autoSave: true);
        });

        var registerInput = new RegisterWorkspaceDto
        {
            Email = registerEmail,
            Password = "SecurePassword123!",
            FirstName = "Brand",
            Surname = "New",
            InviteToken = rawRegisterToken
        };

        // When invoking registration against archived workspace, it throws BusinessException(WorkspaceArchived)
        var registerEx = await Should.ThrowAsync<BusinessException>(async () =>
        {
            await _workspaceAppService.RegisterAsync(registerInput);
        });
        registerEx.Code.ShouldBe(TrackerDomainErrorCodes.WorkspaceArchived);

        // Verify brand_new_signup user was NOT created in DB
        (await _userManager.FindByEmailAsync(registerEmail)).ShouldBeNull();
    }
}
