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
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>
/// REQ-WS-02 / REQ-WS-03 / REQ-ISO-05: Hosted HTTP workspace creation and mine-list lifecycle acceptance suite.
/// Uses the hosted ASP.NET Core TestServer MVC pipeline, disposable project-local SQLite,
/// real production WorkspacesController, WorkspaceAppService, WorkspaceManager,
/// WorkspaceResolutionMiddleware, and real membership/subscription persistence.
/// </summary>
public class WorkspaceCreationLifecycleHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid UserA = Guid.Parse("6f701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid OwnerB = Guid.Parse("6f701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid UserC = Guid.Parse("6f701e62-0953-4dd3-910b-dc6cc93ccb03");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceCreationLifecycleHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
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

    protected static new HttpRequestMessage CreateRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        Guid? workspaceId,
        object? jsonBody = null)
    {
        var req = WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        return req;
    }

    protected static new Guid ReadGuidProperty(JsonElement element, string propName)
    {
        if (element.TryGetProperty(propName, out var p) ||
            element.TryGetProperty(propName.ToLowerInvariant(), out p) ||
            element.TryGetProperty(char.ToUpperInvariant(propName[0]) + propName[1..], out p))
        {
            return Guid.Parse(p.GetString()!);
        }
        throw new KeyNotFoundException($"Property '{propName}' not found in JSON payload: {element.GetRawText()}");
    }

    protected static new string? ReadStringProperty(JsonElement element, string propName)
    {
        if (element.TryGetProperty(propName, out var p) ||
            element.TryGetProperty(propName.ToLowerInvariant(), out p) ||
            element.TryGetProperty(char.ToUpperInvariant(propName[0]) + propName[1..], out p))
        {
            return p.ValueKind == JsonValueKind.Null ? null : p.GetString();
        }
        throw new KeyNotFoundException($"Property '{propName}' not found in JSON payload: {element.GetRawText()}");
    }

    private async Task EnsureUserAsync(Guid id, string userName, string email)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(id.ToString()) == null)
            {
                var user = new IdentityUser(id, userName, email);
                (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
            }
        });
    }

    private async Task EnsureUsersAsync()
    {
        await EnsureUserAsync(UserA, "userA", "userA@acme.test");
        await EnsureUserAsync(OwnerB, "ownerB", "ownerB@acme.test");
        await EnsureUserAsync(UserC, "userC", "userC@acme.test");
    }

    private async Task<(Plan FreePlan, Plan StarterPlan, Plan BusinessPlan)> EnsureDefaultPlansAsync()
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            var free = await _planRepository.FirstOrDefaultAsync(p => p.Name == Plan.FreePlanName);
            if (free == null)
            {
                free = await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), Plan.FreePlanName, 0m, maxSeats: 3, trialDays: 0), autoSave: true);
            }

            var starter = await _planRepository.FirstOrDefaultAsync(p => p.Name == "Starter");
            if (starter == null)
            {
                starter = await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Starter", 6m, maxSeats: 25, trialDays: 14), autoSave: true);
            }

            var business = await _planRepository.FirstOrDefaultAsync(p => p.Name == "Business");
            if (business == null)
            {
                business = await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Business", 12m, maxSeats: 100, trialDays: 14), autoSave: true);
            }

            return (free, starter, business);
        });
    }

    private async Task<Plan> EnsurePaidNoTrialPlanAsync()
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            var existing = await _planRepository.FirstOrDefaultAsync(p => p.Name == "Enterprise-No-Trial");
            if (existing != null)
            {
                return existing;
            }
            var plan = new Plan(_guidGenerator.Create(), "Enterprise-No-Trial", 99m, maxSeats: 50, trialDays: 0);
            return await _planRepository.InsertAsync(plan, autoSave: true);
        });
    }

    private async Task<(int Workspaces, int Members, int Subscriptions)> GetCountsAsync()
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var ws = await _workspaceRepository.CountAsync();
                var members = await _memberRepository.CountAsync();
                var subs = await _subscriptionRepository.CountAsync();
                return (ws, members, subs);
            }
        });
    }

    [Fact]
    public async Task Authenticated_User_Without_Existing_Workspace_Can_Create_Workspace_And_Becomes_Persisted_Active_Owner_With_Subscription()
    {
        await EnsureUsersAsync();
        var (freePlan, _, _) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. UserA has no prior workspace or header; calls POST /api/app/workspaces
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Alpha Engineering",
            PlanId = null // Defaults to free plan
        });
        var createResp = await client.SendAsync(createReq);

        createResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var createJson = await createResp.Content.ReadAsStringAsync();
        using var createDoc = JsonDocument.Parse(createJson);
        var createdId = ReadGuidProperty(createDoc.RootElement, "id");
        ReadStringProperty(createDoc.RootElement, "name").ShouldBe("Alpha Engineering");

        // 2. Real SQLite database assertions for workspace, active owner membership, and subscription
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(createdId);
            ws.Name.ShouldBe("Alpha Engineering");
            ws.NormalizedName.ShouldBe("ALPHA ENGINEERING");
            ws.IsArchived.ShouldBeFalse();

            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == createdId && m.UserId == UserA);
            member.ShouldNotBeNull();
            member.Role.ShouldBe(WorkspaceRoles.Owner);
            member.Status.ShouldBe(WorkspaceMemberStatus.Active);
            member.IsActive.ShouldBeTrue();
            member.IsOwner.ShouldBeTrue();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var sub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == createdId);
                sub.ShouldNotBeNull();
                sub.PlanId.ShouldBe(freePlan.Id);
                sub.Status.ShouldBe(Subscription.ActiveStatus);
            }
        });

        // 3. UserA calls GET /api/app/workspaces/mine
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: UserA, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var mineJson = await mineResp.Content.ReadAsStringAsync();
        using var mineDoc = JsonDocument.Parse(mineJson);
        var mineArray = mineDoc.RootElement.EnumerateArray().ToList();
        mineArray.Count.ShouldBe(1);

        var entry = mineArray[0];
        ReadGuidProperty(entry, "id").ShouldBe(createdId);
        ReadStringProperty(entry, "name").ShouldBe("Alpha Engineering");
        ReadStringProperty(entry, "role").ShouldBe(WorkspaceRoles.Owner);
        entry.GetProperty("isOwner").GetBoolean().ShouldBeTrue();
        entry.GetProperty("isArchived").GetBoolean().ShouldBeFalse();
        ReadStringProperty(entry, "planName").ShouldBe(Plan.FreePlanName);
    }

    [Fact]
    public async Task Same_Account_Can_Create_Second_Distinct_Workspace_And_Mine_Returns_Both_Authorized_Workspaces()
    {
        await EnsureUsersAsync();
        var (freePlan, starterPlan, _) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. UserA creates first workspace "Alpha Engineering"
        var createAlphaReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Alpha Engineering",
            PlanId = freePlan.Id
        });
        var alphaResp = await client.SendAsync(createAlphaReq);
        alphaResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var alphaDoc = JsonDocument.Parse(await alphaResp.Content.ReadAsStringAsync());
        var alphaId = ReadGuidProperty(alphaDoc.RootElement, "id");

        // 2. SAME UserA creates second distinct workspace "Beta Innovations" with Starter plan
        var createBetaReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Beta Innovations",
            PlanId = starterPlan.Id
        });
        var betaResp = await client.SendAsync(createBetaReq);
        betaResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var betaDoc = JsonDocument.Parse(await betaResp.Content.ReadAsStringAsync());
        var betaId = ReadGuidProperty(betaDoc.RootElement, "id");

        betaId.ShouldNotBe(alphaId);

        // 3. Verify SQLite DB has both distinct workspaces and isolated subscriptions
        await WithUnitOfWorkAsync(async () =>
        {
            var alphaWs = await _workspaceRepository.GetAsync(alphaId);
            alphaWs.Name.ShouldBe("Alpha Engineering");
            var betaWs = await _workspaceRepository.GetAsync(betaId);
            betaWs.Name.ShouldBe("Beta Innovations");

            var alphaMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == alphaId && m.UserId == UserA);
            alphaMember.ShouldNotBeNull();
            alphaMember.Role.ShouldBe(WorkspaceRoles.Owner);

            var betaMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == betaId && m.UserId == UserA);
            betaMember.ShouldNotBeNull();
            betaMember.Role.ShouldBe(WorkspaceRoles.Owner);

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var alphaSub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == alphaId);
                alphaSub.ShouldNotBeNull();
                alphaSub.PlanId.ShouldBe(freePlan.Id);

                var betaSub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == betaId);
                betaSub.ShouldNotBeNull();
                betaSub.PlanId.ShouldBe(starterPlan.Id);
            }
        });

        // 4. GET /api/app/workspaces/mine returns exactly the 2 authorized workspaces for UserA
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: UserA, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        using var mineDoc = JsonDocument.Parse(await mineResp.Content.ReadAsStringAsync());
        var mineItems = mineDoc.RootElement.EnumerateArray().ToList();
        mineItems.Count.ShouldBe(2);

        // Sorted by name ascending (OrdinalIgnoreCase)
        var item0 = mineItems[0];
        ReadGuidProperty(item0, "id").ShouldBe(alphaId);
        ReadStringProperty(item0, "name").ShouldBe("Alpha Engineering");
        ReadStringProperty(item0, "role").ShouldBe(WorkspaceRoles.Owner);
        item0.GetProperty("isOwner").GetBoolean().ShouldBeTrue();
        ReadStringProperty(item0, "planName").ShouldBe(Plan.FreePlanName);

        var item1 = mineItems[1];
        ReadGuidProperty(item1, "id").ShouldBe(betaId);
        ReadStringProperty(item1, "name").ShouldBe("Beta Innovations");
        ReadStringProperty(item1, "role").ShouldBe(WorkspaceRoles.Owner);
        item1.GetProperty("isOwner").GetBoolean().ShouldBeTrue();
        ReadStringProperty(item1, "planName").ShouldBe("Starter");
    }

    [Fact]
    public async Task Another_Account_And_Real_Foreign_Workspace_Sentinels_Remain_Isolated_And_Absent_From_Mine()
    {
        await EnsureUsersAsync();
        var (_, _, businessPlan) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Seed foreign workspace B for OwnerB with Business subscription and sentinel Project
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Zeta Holding");
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerB, WorkspaceRoles.Owner, wsB);

        var sentinelProjectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var subB = new Subscription(_guidGenerator.Create(), wsB, businessPlan.Id)
                {
                    Status = Subscription.ActiveStatus
                };
                await _subscriptionRepository.InsertAsync(subB, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var projectB = new Project(sentinelProjectId, wsB, "Sentinel-Project-B", color: "#223344");
                await _projectRepository.InsertAsync(projectB, autoSave: true);
            }
        });

        // 2. UserA creates "Alpha Engineering"
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Alpha Engineering"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var createDoc = JsonDocument.Parse(await createResp.Content.ReadAsStringAsync());
        var wsA = ReadGuidProperty(createDoc.RootElement, "id");

        // 3. UserA calls GET /api/app/workspaces/mine: wsB MUST be absent
        var mineReqA = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: UserA, workspaceId: null);
        var mineRespA = await client.SendAsync(mineReqA);
        mineRespA.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var mineDocA = JsonDocument.Parse(await mineRespA.Content.ReadAsStringAsync());
        var userAItems = mineDocA.RootElement.EnumerateArray().ToList();
        userAItems.Count.ShouldBe(1);
        ReadGuidProperty(userAItems[0], "id").ShouldBe(wsA);
        userAItems.Any(i => ReadGuidProperty(i, "id") == wsB).ShouldBeFalse();

        // 4. OwnerB calls GET /api/app/workspaces/mine: wsA MUST be absent
        var mineReqB = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: OwnerB, workspaceId: null);
        var mineRespB = await client.SendAsync(mineReqB);
        mineRespB.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var mineDocB = JsonDocument.Parse(await mineRespB.Content.ReadAsStringAsync());
        var ownerBItems = mineDocB.RootElement.EnumerateArray().ToList();
        ownerBItems.Count.ShouldBe(1);
        ReadGuidProperty(ownerBItems[0], "id").ShouldBe(wsB);
        ownerBItems.Any(i => ReadGuidProperty(i, "id") == wsA).ShouldBeFalse();

        // 5. In explicit correct context (disabling workspace filter), foreign sentinels remain 100% intact
        await WithUnitOfWorkAsync(async () =>
        {
            var wsBEntity = await _workspaceRepository.GetAsync(wsB);
            wsBEntity.Name.ShouldBe("Zeta Holding");

            var memberB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == OwnerB);
            memberB.ShouldNotBeNull();
            memberB.Role.ShouldBe(WorkspaceRoles.Owner);

            // UserA is NOT a member of Workspace B
            var memberAInB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == UserA);
            memberAInB.ShouldBeNull();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var subB = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == wsB);
                subB.ShouldNotBeNull();
                subB.PlanId.ShouldBe(businessPlan.Id);

                var projB = await _projectRepository.FirstOrDefaultAsync(p => p.Id == sentinelProjectId && p.WorkspaceId == wsB);
                projB.ShouldNotBeNull();
                projB.Title.ShouldBe("Sentinel-Project-B");
            }
        });
    }

    [Fact]
    public async Task Unauthenticated_Requests_Are_Denied_With_401_Unauthorized_And_No_Persisted_Writes()
    {
        await EnsureUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        var (wsCountBefore, memberCountBefore, subCountBefore) = await GetCountsAsync();

        // 1. Unauthenticated POST /api/app/workspaces
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: null, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Unauthorized Corp"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        var createBody = await createResp.Content.ReadAsStringAsync();
        createBody.ShouldContain("Volo.Abp.Authorization:010001");

        // 2. Unauthenticated GET /api/app/workspaces/mine
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: null, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        var mineBody = await mineResp.Content.ReadAsStringAsync();
        mineBody.ShouldContain("Volo.Abp.Authorization:010001");

        // 3. Verify zero writes across all relevant tables
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "UNAUTHORIZED CORP");
            ws.ShouldBeNull();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _workspaceRepository.CountAsync()).ShouldBe(wsCountBefore);
                (await _memberRepository.CountAsync()).ShouldBe(memberCountBefore);
                (await _subscriptionRepository.CountAsync()).ShouldBe(subCountBefore);
            }
        });
    }

    [Fact]
    public async Task Invalid_Name_Is_Rejected_With_400_BadRequest_Without_Partial_Writes()
    {
        await EnsureUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        var (wsCountBefore, memberCountBefore, subCountBefore) = await GetCountsAsync();

        // 1. Empty string
        var emptyReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = ""
        });
        var emptyResp = await client.SendAsync(emptyReq);
        emptyResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 2. Whitespace-only string
        var wsOnlyReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "   "
        });
        var wsOnlyResp = await client.SendAsync(wsOnlyReq);
        wsOnlyResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Name exceeding MaxNameLength (64 characters)
        var overlongReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = new string('A', 65)
        });
        var overlongResp = await client.SendAsync(overlongReq);
        overlongResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 4. Verify negative persistence: zero partial workspace, member, or subscription created
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _workspaceRepository.CountAsync()).ShouldBe(wsCountBefore);
                (await _memberRepository.CountAsync()).ShouldBe(memberCountBefore);
                (await _subscriptionRepository.CountAsync()).ShouldBe(subCountBefore);
            }
        });
    }

    [Fact]
    public async Task Duplicate_Normalized_Name_Is_Rejected_With_409_Conflict_Without_Partial_Writes()
    {
        await EnsureUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. First creation succeeds
        var firstReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Alpha Engineering"
        });
        var firstResp = await client.SendAsync(firstReq);
        firstResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var (wsCountAfterFirst, memberCountAfterFirst, subCountAfterFirst) = await GetCountsAsync();

        // 2. Duplicate with whitespace padding and lower case
        var dupReq1 = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: OwnerB, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "  alpha engineering  "
        });
        var dupResp1 = await client.SendAsync(dupReq1);
        dupResp1.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var body1 = await dupResp1.Content.ReadAsStringAsync();
        body1.ShouldContain(TrackerDomainErrorCodes.DuplicateWorkspaceName);

        // 3. Duplicate with all UPPERCASE
        var dupReq2 = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "ALPHA ENGINEERING"
        });
        var dupResp2 = await client.SendAsync(dupReq2);
        dupResp2.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var body2 = await dupResp2.Content.ReadAsStringAsync();
        body2.ShouldContain(TrackerDomainErrorCodes.DuplicateWorkspaceName);

        // 4. Verify negative persistence: counts strictly unchanged from after the first valid creation
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _workspaceRepository.CountAsync()).ShouldBe(wsCountAfterFirst);
                (await _memberRepository.CountAsync()).ShouldBe(memberCountAfterFirst);
                (await _subscriptionRepository.CountAsync()).ShouldBe(subCountAfterFirst);
            }
        });
    }

    [Fact]
    public async Task Plan_Selection_Enforces_Contract_Invalid_Plan_Returns_404_And_Paid_Without_Trial_Returns_400()
    {
        await EnsureUsersAsync();
        await EnsureDefaultPlansAsync();
        var paidNoTrialPlan = await EnsurePaidNoTrialPlanAsync();
        var client = CreateHttpClient();

        var (wsCountBefore, memberCountBefore, subCountBefore) = await GetCountsAsync();

        // 1. Non-existent Plan GUID returns 404 NotFound (TrackerDomainErrorCodes.PlanNotFound)
        var nonExistentPlanId = Guid.NewGuid();
        var badPlanReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Unknown Plan Workspace",
            PlanId = nonExistentPlanId
        });
        var badPlanResp = await client.SendAsync(badPlanReq);
        badPlanResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        var badPlanBody = await badPlanResp.Content.ReadAsStringAsync();
        badPlanBody.ShouldContain(TrackerDomainErrorCodes.PlanNotFound);

        // 2. Paid plan with zero trial days returns 400 BadRequest (AbpValidationException)
        var paidNoTrialReq = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: null, jsonBody: new CreateWorkspaceDto
        {
            Name = "Paid No Trial Workspace",
            PlanId = paidNoTrialPlan.Id
        });
        var paidNoTrialResp = await client.SendAsync(paidNoTrialReq);
        paidNoTrialResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Verify negative persistence: neither workspace was partially created
        await WithUnitOfWorkAsync(async () =>
        {
            (await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "UNKNOWN PLAN WORKSPACE")).ShouldBeNull();
            (await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "PAID NO TRIAL WORKSPACE")).ShouldBeNull();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _workspaceRepository.CountAsync()).ShouldBe(wsCountBefore);
                (await _memberRepository.CountAsync()).ShouldBe(memberCountBefore);
                (await _subscriptionRepository.CountAsync()).ShouldBe(subCountBefore);
            }
        });
    }

    [Fact]
    public async Task Supplied_Ambient_Or_Foreign_Header_Cannot_Hijack_Ownership_Or_Mutate_Existing_Foreign_State()
    {
        await EnsureUsersAsync();
        var (freePlan, _, businessPlan) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Seed Workspace B owned by OwnerB
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Zeta Holding");
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerB, WorkspaceRoles.Owner, wsB);

        // UserA is an active Member (NOT Owner) of Workspace B
        await ServiceProvider.AddWorkspaceMemberAsync(UserA, WorkspaceRoles.Member, wsB);

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var subB = new Subscription(_guidGenerator.Create(), wsB, businessPlan.Id)
                {
                    Status = Subscription.ActiveStatus
                };
                await _subscriptionRepository.InsertAsync(subB, autoSave: true);
            }
        });

        // 2. Subcase A: UserA supplies ambient header X-Workspace-Id: {wsB} (where UserA is member) while creating "Gamma Global"
        // Since WorkspacesController is marked [WorkspaceNotRequired], the endpoint succeeds,
        // but it MUST NOT elevate UserA in Workspace B or mutate Workspace B.
        var reqWithAmbientHeader = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: wsB, jsonBody: new CreateWorkspaceDto
        {
            Name = "Gamma Global",
            PlanId = freePlan.Id
        });
        var respWithAmbientHeader = await client.SendAsync(reqWithAmbientHeader);
        respWithAmbientHeader.StatusCode.ShouldBe(HttpStatusCode.OK);

        using var gammaDoc = JsonDocument.Parse(await respWithAmbientHeader.Content.ReadAsStringAsync());
        var gammaId = ReadGuidProperty(gammaDoc.RootElement, "id");
        gammaId.ShouldNotBe(wsB);

        // Assertions on Gamma Global vs Workspace B
        await WithUnitOfWorkAsync(async () =>
        {
            // UserA is Owner of the new Gamma Global workspace
            var gammaMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == gammaId && m.UserId == UserA);
            gammaMember.ShouldNotBeNull();
            gammaMember.Role.ShouldBe(WorkspaceRoles.Owner);

            // Workspace B remains completely untouched: OwnerB is STILL owner, UserA is STILL member (not owner)
            var ownerBMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == OwnerB);
            ownerBMember.ShouldNotBeNull();
            ownerBMember.Role.ShouldBe(WorkspaceRoles.Owner);

            var userAMemberInB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == UserA);
            userAMemberInB.ShouldNotBeNull();
            userAMemberInB.Role.ShouldBe(WorkspaceRoles.Member);

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var subB = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == wsB);
                subB.ShouldNotBeNull();
                subB.PlanId.ShouldBe(businessPlan.Id);
            }
        });

        // 3. Subcase B: Seed Workspace C owned by UserC where UserA is NOT a member at all
        var wsC = await ServiceProvider.CreateWorkspaceAsync("Omega Subsidiary");
        await ServiceProvider.AddWorkspaceMemberAsync(UserC, WorkspaceRoles.Owner, wsC);

        int wsCountBeforeSubcaseB;
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            wsCountBeforeSubcaseB = (await GetCountsAsync()).Workspaces;
        }

        // UserA attempts to create "Delta Corp" while providing foreign header X-Workspace-Id: {wsC}
        // WorkspaceResolutionMiddleware intercepts this because UserA is not a member of wsC -> returns 403 Forbidden
        var reqWithForeignHeader = CreateRequest(HttpMethod.Post, "/api/app/workspaces", userId: UserA, workspaceId: wsC, jsonBody: new CreateWorkspaceDto
        {
            Name = "Delta Corp"
        });
        var respWithForeignHeader = await client.SendAsync(reqWithForeignHeader);
        respWithForeignHeader.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var foreignBody = await respWithForeignHeader.Content.ReadAsStringAsync();
        foreignBody.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Verify zero writes occurred and Workspace C was not modified
        await WithUnitOfWorkAsync(async () =>
        {
            (await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "DELTA CORP")).ShouldBeNull();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _workspaceRepository.CountAsync()).ShouldBe(wsCountBeforeSubcaseB);

                var wsCEntity = await _workspaceRepository.GetAsync(wsC);
                wsCEntity.Name.ShouldBe("Omega Subsidiary");

                var userCMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsC && m.UserId == UserC);
                userCMember.ShouldNotBeNull();
                userCMember.Role.ShouldBe(WorkspaceRoles.Owner);

                var userAMemberInC = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsC && m.UserId == UserA);
                userAMemberInC.ShouldBeNull();
            }
        });
    }
}
