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
/// REQ-WS-01 / REQ-ISO-05: Hosted HTTP anonymous workspace signup lifecycle and invitation-registration isolation acceptance suite.
/// Uses the hosted ASP.NET Core TestServer MVC pipeline, disposable project-local SQLite,
/// real production WorkspaceAppService (conventional route POST /api/app/workspace/register), WorkspacesController (GET /mine),
/// WorkspaceManager, WorkspaceMembershipManager, TrackerAccountFactory/Identity persistence,
/// WorkspaceResolutionMiddleware, and real membership/subscription persistence.
/// </summary>
public class WorkspaceRegistrationLifecycleHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid ForeignOwner = Guid.Parse("7f701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ForeignMember = Guid.Parse("7f701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid ExistingUser = Guid.Parse("7f701e62-0953-4dd3-910b-dc6cc93ccb03");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IIdentityUserRepository _identityUserRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceRegistrationLifecycleHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _invitationRepository = GetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _identityUserRepository = GetRequiredService<IIdentityUserRepository>();
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
        services.AddControllers()
            .AddApplicationPart(typeof(WorkspacesController).Assembly)
            .AddApplicationPart(typeof(WorkspaceAppService).Assembly);
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
                (await _userManager.CreateAsync(user, "Password123!")).Succeeded.ShouldBeTrue();
            }
        });
    }

    private async Task EnsureInitialSeedUsersAsync()
    {
        await EnsureUserAsync(ForeignOwner, "foreign_owner", "foreign.owner@isolated.test");
        await EnsureUserAsync(ForeignMember, "foreign_member", "foreign.member@isolated.test");
        await EnsureUserAsync(ExistingUser, "existing_user", "existing@isolated.test");
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

    private async Task<(int Users, int Workspaces, int Members, int Subscriptions, int Invitations)> GetCountsAsync()
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var users = (int)await _identityUserRepository.GetCountAsync();
                var ws = await _workspaceRepository.CountAsync();
                var members = await _memberRepository.CountAsync();
                var subs = await _subscriptionRepository.CountAsync();
                var invs = await _invitationRepository.CountAsync();
                return (users, ws, members, subs, invs);
            }
        });
    }

    [Fact]
    public async Task Anonymous_Ordinary_Signup_Persists_Identity_Owner_Workspace_And_Permitted_Subscription_And_Mine_Lists_Workspace()
    {
        await EnsureInitialSeedUsersAsync();
        var (freePlan, _, _) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Anonymous caller posts to conventional POST /api/app/workspace/register without headers
        var signupDto = new RegisterWorkspaceDto
        {
            Email = "alpha.founder@signup-test.com",
            Password = "Password123!",
            FirstName = "Alpha",
            Surname = "Founder",
            WorkspaceName = "Alpha Technologies",
            PlanId = null // Defaults to free plan
        };
        var signupReq = CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: signupDto);
        var signupResp = await client.SendAsync(signupReq);

        signupResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var signupJson = await signupResp.Content.ReadAsStringAsync();
        using var signupDoc = JsonDocument.Parse(signupJson);
        var createdWsId = ReadGuidProperty(signupDoc.RootElement, "workspaceId");
        ReadStringProperty(signupDoc.RootElement, "workspaceName").ShouldBe("Alpha Technologies");
        var userName = ReadStringProperty(signupDoc.RootElement, "userName");
        userName.ShouldBe("alpha.founder@signup-test.com");

        // 2. Real SQLite database assertions for Identity user, workspace, active owner membership, and subscription
        Guid createdUserId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            var user = await _userManager.FindByEmailAsync("alpha.founder@signup-test.com");
            user.ShouldNotBeNull();
            user.Name.ShouldBe("Alpha");
            user.Surname.ShouldBe("Founder");
            createdUserId = user.Id;

            var ws = await _workspaceRepository.GetAsync(createdWsId);
            ws.Name.ShouldBe("Alpha Technologies");
            ws.NormalizedName.ShouldBe("ALPHA TECHNOLOGIES");
            ws.IsArchived.ShouldBeFalse();

            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == createdWsId && m.UserId == createdUserId);
            member.ShouldNotBeNull();
            member.Role.ShouldBe(WorkspaceRoles.Owner);
            member.Status.ShouldBe(WorkspaceMemberStatus.Active);
            member.IsActive.ShouldBeTrue();
            member.IsOwner.ShouldBeTrue();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var sub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == createdWsId);
                sub.ShouldNotBeNull();
                sub.PlanId.ShouldBe(freePlan.Id);
                sub.Status.ShouldBe(Subscription.ActiveStatus);
            }
        });

        // 3. Obtain the new persisted identity in the existing synthetic-auth harness;
        // fresh /mine lists exactly its authorized workspace.
        // NOTE: This proves aggregate authorization and persistence, NOT real token issuance/login.
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: createdUserId, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var mineJson = await mineResp.Content.ReadAsStringAsync();
        using var mineDoc = JsonDocument.Parse(mineJson);
        var mineArray = mineDoc.RootElement.EnumerateArray().ToList();
        mineArray.Count.ShouldBe(1);

        var entry = mineArray[0];
        ReadGuidProperty(entry, "id").ShouldBe(createdWsId);
        ReadStringProperty(entry, "name").ShouldBe("Alpha Technologies");
        ReadStringProperty(entry, "role").ShouldBe(WorkspaceRoles.Owner);
        entry.GetProperty("isOwner").GetBoolean().ShouldBeTrue();
        entry.GetProperty("isArchived").GetBoolean().ShouldBeFalse();
        ReadStringProperty(entry, "planName").ShouldBe(Plan.FreePlanName);
    }

    [Fact]
    public async Task Anonymous_Ordinary_Signup_With_Explicit_Starter_Plan_Persists_Trialing_Subscription()
    {
        await EnsureInitialSeedUsersAsync();
        var (_, starterPlan, _) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        var signupDto = new RegisterWorkspaceDto
        {
            Email = "beta.founder@signup-test.com",
            Password = "Password123!",
            FirstName = "Beta",
            Surname = "Founder",
            WorkspaceName = "Beta Innovations",
            PlanId = starterPlan.Id
        };
        var signupReq = CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: signupDto);
        var signupResp = await client.SendAsync(signupReq);

        signupResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        using var signupDoc = JsonDocument.Parse(await signupResp.Content.ReadAsStringAsync());
        var createdWsId = ReadGuidProperty(signupDoc.RootElement, "workspaceId");

        // Verify trialing subscription in SQLite
        Guid userId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            var user = await _userManager.FindByEmailAsync("beta.founder@signup-test.com");
            user.ShouldNotBeNull();
            userId = user.Id;

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var sub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == createdWsId);
                sub.ShouldNotBeNull();
                sub.PlanId.ShouldBe(starterPlan.Id);
                sub.Status.ShouldBe(Subscription.TrialingStatus);
                sub.TrialEndsAt.ShouldNotBeNull();
                (sub.TrialEndsAt.Value > DateTime.UtcNow).ShouldBeTrue();
            }
        });

        // Fresh /mine listing reports Starter plan
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: userId, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        using var mineDoc = JsonDocument.Parse(await mineResp.Content.ReadAsStringAsync());
        var mineArray = mineDoc.RootElement.EnumerateArray().ToList();
        mineArray.Count.ShouldBe(1);
        ReadStringProperty(mineArray[0], "planName").ShouldBe("Starter");
        ReadStringProperty(mineArray[0], "role").ShouldBe(WorkspaceRoles.Owner);
    }

    [Fact]
    public async Task Duplicate_Email_And_Duplicate_Normalized_Workspace_Name_Are_Rejected_With_409_Conflict_Without_Partial_Writes()
    {
        await EnsureInitialSeedUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Initial valid registration
        var firstDto = new RegisterWorkspaceDto
        {
            Email = "unique.founder@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "Unique Corp"
        };
        var firstResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: firstDto));
        firstResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var (usersAfterFirst, wsAfterFirst, membersAfterFirst, subsAfterFirst, invsAfterFirst) = await GetCountsAsync();

        // 2. Subcase A: Duplicate email (TrackerDomainErrorCodes.EmailAlreadyRegistered)
        var dupEmailDto = new RegisterWorkspaceDto
        {
            Email = "unique.founder@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "Different Workspace Co"
        };
        var dupEmailResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: dupEmailDto));
        dupEmailResp.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var dupEmailBody = await dupEmailResp.Content.ReadAsStringAsync();
        dupEmailBody.ShouldContain(TrackerDomainErrorCodes.EmailAlreadyRegistered);

        // Verify zero partial writes after duplicate email rejection
        var countsAfterDupEmail = await GetCountsAsync();
        countsAfterDupEmail.Users.ShouldBe(usersAfterFirst);
        countsAfterDupEmail.Workspaces.ShouldBe(wsAfterFirst);
        countsAfterDupEmail.Members.ShouldBe(membersAfterFirst);
        countsAfterDupEmail.Subscriptions.ShouldBe(subsAfterFirst);
        await WithUnitOfWorkAsync(async () =>
        {
            (await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "DIFFERENT WORKSPACE CO")).ShouldBeNull();
        });

        // 3. Subcase B: Duplicate normalized workspace name (whitespace-padded & lowercase)
        var dupNameDto = new RegisterWorkspaceDto
        {
            Email = "another.founder@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "   unique corp   "
        };
        var dupNameResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: dupNameDto));
        dupNameResp.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var dupNameBody = await dupNameResp.Content.ReadAsStringAsync();
        dupNameBody.ShouldContain(TrackerDomainErrorCodes.DuplicateWorkspaceName);

        // Verify zero partial writes after duplicate workspace name rejection
        var countsAfterDupName = await GetCountsAsync();
        countsAfterDupName.Users.ShouldBe(usersAfterFirst);
        countsAfterDupName.Workspaces.ShouldBe(wsAfterFirst);
        countsAfterDupName.Members.ShouldBe(membersAfterFirst);
        countsAfterDupName.Subscriptions.ShouldBe(subsAfterFirst);
        await WithUnitOfWorkAsync(async () =>
        {
            (await _userManager.FindByEmailAsync("another.founder@signup-test.com")).ShouldBeNull();
        });
    }

    [Fact]
    public async Task Weak_Password_And_Invalid_Inputs_Are_Rejected_With_400_BadRequest_Without_Partial_Writes()
    {
        await EnsureInitialSeedUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        var (usersBefore, wsBefore, membersBefore, subsBefore, invsBefore) = await GetCountsAsync();

        // 1. Weak password rejected by Identity password validator
        var weakPwDto = new RegisterWorkspaceDto
        {
            Email = "weak.pw@signup-test.com",
            Password = "123",
            WorkspaceName = "Weak Pw Co"
        };
        var weakPwResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: weakPwDto));
        weakPwResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 2. Missing workspace name when no invite token (IValidatableObject)
        var missingWsDto = new RegisterWorkspaceDto
        {
            Email = "missing.ws@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "   "
        };
        var missingWsResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: missingWsDto));
        missingWsResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Overlong workspace name (>64 chars)
        var overlongWsDto = new RegisterWorkspaceDto
        {
            Email = "overlong.ws@signup-test.com",
            Password = "Password123!",
            WorkspaceName = new string('A', 65)
        };
        var overlongWsResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: overlongWsDto));
        overlongWsResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 4. Invalid email format
        var invalidEmailDto = new RegisterWorkspaceDto
        {
            Email = "not-an-email",
            Password = "Password123!",
            WorkspaceName = "Invalid Email Co"
        };
        var invalidEmailResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: invalidEmailDto));
        invalidEmailResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 5. Verify zero partial writes across all failed validations
        var (usersAfter, wsAfter, membersAfter, subsAfter, invsAfter) = await GetCountsAsync();
        usersAfter.ShouldBe(usersBefore);
        wsAfter.ShouldBe(wsBefore);
        membersAfter.ShouldBe(membersBefore);
        subsAfter.ShouldBe(subsBefore);
        invsAfter.ShouldBe(invsBefore);
    }

    [Fact]
    public async Task Plan_Contract_Enforcement_Invalid_Plan_Returns_404_And_Paid_Without_Trial_Returns_400()
    {
        await EnsureInitialSeedUsersAsync();
        await EnsureDefaultPlansAsync();
        var paidNoTrialPlan = await EnsurePaidNoTrialPlanAsync();
        var client = CreateHttpClient();

        var (usersBefore, wsBefore, membersBefore, subsBefore, invsBefore) = await GetCountsAsync();

        // 1. Non-existent Plan GUID returns 404 NotFound (TrackerDomainErrorCodes.PlanNotFound)
        var nonExistentPlanDto = new RegisterWorkspaceDto
        {
            Email = "badplan@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "Bad Plan Co",
            PlanId = Guid.NewGuid()
        };
        var badPlanResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: nonExistentPlanDto));
        badPlanResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await badPlanResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.PlanNotFound);

        // 2. Paid plan with zero trial days returns 400 BadRequest (AbpValidationException)
        var noTrialDto = new RegisterWorkspaceDto
        {
            Email = "notrial@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "No Trial Co",
            PlanId = paidNoTrialPlan.Id
        };
        var noTrialResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: noTrialDto));
        noTrialResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Verify zero partial writes
        var (usersAfter, wsAfter, membersAfter, subsAfter, invsAfter) = await GetCountsAsync();
        usersAfter.ShouldBe(usersBefore);
        wsAfter.ShouldBe(wsBefore);
        membersAfter.ShouldBe(membersBefore);
        subsAfter.ShouldBe(subsBefore);
    }

    [Fact]
    public async Task Direct_Call_To_Register_With_Invite()
    {
        await EnsureInitialSeedUsersAsync();
        var (_, _, businessPlan) = await EnsureDefaultPlansAsync();
        var db = ServiceProvider.GetRequiredService<TrackerDbContext>();

        var wsB = await ServiceProvider.CreateWorkspaceAsync("Echo Enterprise Direct");
        var s1 = (await db.Set<Workspace>().AsNoTracking().FirstAsync(w => w.Id == wsB)).ConcurrencyStamp;

        await ServiceProvider.AddWorkspaceMemberAsync(ForeignOwner, WorkspaceRoles.Owner, wsB);
        var s2 = (await db.Set<Workspace>().AsNoTracking().FirstAsync(w => w.Id == wsB)).ConcurrencyStamp;

        var (tokenA, _) = await SeedInvitationAsync(wsB, "invited.direct@signup-test.com", role: WorkspaceRoles.Member);
        var s3 = (await db.Set<Workspace>().AsNoTracking().FirstAsync(w => w.Id == wsB)).ConcurrencyStamp;

        var appService = GetRequiredService<IWorkspaceAppService>();
        try
        {
            await appService.RegisterAsync(new RegisterWorkspaceDto
            {
                Email = "invited.direct@signup-test.com",
                Password = "Password123!",
                FirstName = "Invited",
                Surname = "Direct",
                InviteToken = tokenA
            });
        }
        catch (AbpDbConcurrencyException ex)
        {
            if (ex.InnerException is DbUpdateConcurrencyException efEx)
            {
                var currentInDb = await db.Set<Workspace>().AsNoTracking().FirstOrDefaultAsync(w => w.Id == wsB);
                var entry = efEx.Entries.First();
                throw new Exception($"STAMPS: s1={s1}, s2={s2}, s3={s3}, InDb={currentInDb?.ConcurrencyStamp}, OrigEntry={entry.Property("ConcurrencyStamp").OriginalValue}", ex);
            }
            throw;
        }
    }

    [Fact]
    public async Task Invitation_Signup_Joins_Invited_Workspace_With_Assigned_Role_And_Consumes_Invitation_Without_Extra_Workspace_Or_Subscription()
    {
        await EnsureInitialSeedUsersAsync();
        var (_, _, businessPlan) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Seed Workspace B owned by ForeignOwner with Business subscription
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Echo Enterprise");
        await ServiceProvider.AddWorkspaceMemberAsync(ForeignOwner, WorkspaceRoles.Owner, wsB);

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

        // 2. Seed an invitation in Workspace B for "invited.member@signup-test.com" with role "member"
        var (tokenA, invitationA) = await SeedInvitationAsync(wsB, "invited.member@signup-test.com", role: WorkspaceRoles.Member);

        var (usersBefore, wsBefore, membersBefore, subsBefore, invsBefore) = await GetCountsAsync();

        // 3. Invitee registers through the SAME conventional POST /api/app/workspace/register route
        var inviteSignupDto = new RegisterWorkspaceDto
        {
            Email = "invited.member@signup-test.com",
            Password = "Password123!",
            FirstName = "Invited",
            Surname = "Member",
            InviteToken = tokenA
        };
        var inviteSignupResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: inviteSignupDto));
        var inviteSignupBody = await inviteSignupResp.Content.ReadAsStringAsync();
        inviteSignupResp.StatusCode.ShouldBe(HttpStatusCode.OK, customMessage: inviteSignupBody);

        using var resultDoc = JsonDocument.Parse(await inviteSignupResp.Content.ReadAsStringAsync());
        ReadGuidProperty(resultDoc.RootElement, "workspaceId").ShouldBe(wsB);
        ReadStringProperty(resultDoc.RootElement, "workspaceName").ShouldBe("Echo Enterprise");

        // 4. Detailed SQLite persistence verification
        Guid newUserId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            var user = await _userManager.FindByEmailAsync("invited.member@signup-test.com");
            user.ShouldNotBeNull();
            user.Name.ShouldBe("Invited");
            user.Surname.ShouldBe("Member");
            newUserId = user.Id;

            // Workspace count MUST NOT increase (no accidental extra workspace!)
            (await _workspaceRepository.CountAsync()).ShouldBe(wsBefore);

            // Subscription count MUST NOT increase (no accidental extra subscription!)
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                (await _subscriptionRepository.CountAsync()).ShouldBe(subsBefore);
            }

            // Exactly ONE membership created: for wsB with assigned role "member" (NOT owner)
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == newUserId);
            member.ShouldNotBeNull();
            member.Role.ShouldBe(WorkspaceRoles.Member);
            member.IsActive.ShouldBeTrue();
            member.IsOwner.ShouldBeFalse();

            // Invitation is consumed: AcceptedBy set, AcceptedAt set
            var inv = await _invitationRepository.GetAsync(invitationA.Id);
            inv.AcceptedUserId.ShouldBe(newUserId);
            inv.AcceptedAt.ShouldNotBeNull();
            inv.IsUsable(DateTime.UtcNow).ShouldBeFalse();
        });

        // 5. Fresh /mine listing as new invitee user
        var mineReq = CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId: newUserId, workspaceId: null);
        var mineResp = await client.SendAsync(mineReq);
        mineResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        using var mineDoc = JsonDocument.Parse(await mineResp.Content.ReadAsStringAsync());
        var mineArray = mineDoc.RootElement.EnumerateArray().ToList();
        mineArray.Count.ShouldBe(1);
        ReadGuidProperty(mineArray[0], "id").ShouldBe(wsB);
        ReadStringProperty(mineArray[0], "name").ShouldBe("Echo Enterprise");
        ReadStringProperty(mineArray[0], "role").ShouldBe(WorkspaceRoles.Member);
        mineArray[0].GetProperty("isOwner").GetBoolean().ShouldBeFalse();
        ReadStringProperty(mineArray[0], "planName").ShouldBe("Business");
    }

    [Fact]
    public async Task Invitation_Signup_Fails_On_Mismatched_Email_Expired_Declined_Token_Or_Archived_Target_Before_User_Creation()
    {
        await EnsureInitialSeedUsersAsync();
        await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        var wsTarget = await ServiceProvider.CreateWorkspaceAsync("Target Holding");
        await ServiceProvider.AddWorkspaceMemberAsync(ForeignOwner, WorkspaceRoles.Owner, wsTarget);

        var wsArchived = await ServiceProvider.CreateWorkspaceAsync("Archived Target Corp");
        await ServiceProvider.AddWorkspaceMemberAsync(ForeignOwner, WorkspaceRoles.Owner, wsArchived);
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsArchived);
            ws.Archive(ForeignOwner, DateTime.UtcNow);
            await _workspaceRepository.UpdateAsync(ws, autoSave: true);
        });

        // Seed valid invitation for target
        var (tokenMismatched, _) = await SeedInvitationAsync(wsTarget, "recipient.only@signup-test.com");
        // Seed expired invitation
        var (tokenExpired, _) = await SeedInvitationAsync(wsTarget, "expired.invitee@signup-test.com", expiresAt: DateTime.UtcNow.AddDays(-1));
        // Seed invitation for archived workspace
        var (tokenArchived, _) = await SeedInvitationAsync(wsArchived, "archived.invitee@signup-test.com");

        var (usersBefore, wsBefore, membersBefore, subsBefore, invsBefore) = await GetCountsAsync();

        // 1. Mismatched email (403 Tracker:InvitationEmailMismatch)
        var mismatchDto = new RegisterWorkspaceDto
        {
            Email = "imposter@signup-test.com",
            Password = "Password123!",
            InviteToken = tokenMismatched
        };
        var mismatchResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: mismatchDto));
        mismatchResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await mismatchResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationEmailMismatch);

        // 2. Expired invitation token (410 Tracker:InvitationExpired)
        var expiredDto = new RegisterWorkspaceDto
        {
            Email = "expired.invitee@signup-test.com",
            Password = "Password123!",
            InviteToken = tokenExpired
        };
        var expiredResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: expiredDto));
        expiredResp.StatusCode.ShouldBe(HttpStatusCode.Gone);
        (await expiredResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationExpired);

        // 3. Unknown or already-declined token (404 Tracker:InvitationNotFound)
        var unknownDto = new RegisterWorkspaceDto
        {
            Email = "someone@signup-test.com",
            Password = "Password123!",
            InviteToken = "completely-unknown-token-hash-xyz"
        };
        var unknownResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: unknownDto));
        unknownResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await unknownResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.InvitationNotFound);

        // 4. Target workspace is archived (403 Tracker:WorkspaceArchived)
        var archivedDto = new RegisterWorkspaceDto
        {
            Email = "archived.invitee@signup-test.com",
            Password = "Password123!",
            InviteToken = tokenArchived
        };
        var archivedResp = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: null, jsonBody: archivedDto));
        archivedResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await archivedResp.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.WorkspaceArchived);

        // 5. Verify zero user creation or membership writes across all failure branches
        var (usersAfter, wsAfter, membersAfter, subsAfter, invsAfter) = await GetCountsAsync();
        usersAfter.ShouldBe(usersBefore);
        wsAfter.ShouldBe(wsBefore);
        membersAfter.ShouldBe(membersBefore);
        subsAfter.ShouldBe(subsBefore);
    }

    [Fact]
    public async Task Ambient_Workspace_Header_Precedence_Does_Not_Mutate_Foreign_Workspace_And_Malformed_Header_Fails_Closed()
    {
        await EnsureInitialSeedUsersAsync();
        var (freePlan, _, businessPlan) = await EnsureDefaultPlansAsync();
        var client = CreateHttpClient();

        // 1. Seed foreign workspace with user, member, subscription, project, and separate pending invitation
        var wsForeign = await ServiceProvider.CreateWorkspaceAsync("Foreign Sentinel Corp");
        await ServiceProvider.AddWorkspaceMemberAsync(ForeignOwner, WorkspaceRoles.Owner, wsForeign);
        await ServiceProvider.AddWorkspaceMemberAsync(ForeignMember, WorkspaceRoles.Member, wsForeign);

        var sentinelProjId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var subForeign = new Subscription(_guidGenerator.Create(), wsForeign, businessPlan.Id)
                {
                    Status = Subscription.ActiveStatus
                };
                await _subscriptionRepository.InsertAsync(subForeign, autoSave: true);
            }

            using (_currentWorkspace.Change(wsForeign, WorkspaceRoles.Owner))
            {
                var project = new Project(sentinelProjId, wsForeign, "Foreign-Sentinel-Project", color: "#113355");
                await _projectRepository.InsertAsync(project, autoSave: true);
            }
        });

        var (tokenB, invitationB) = await SeedInvitationAsync(wsForeign, "foreign.invitee@isolated.test", role: WorkspaceRoles.Member);

        // 2. Subcase A: Anonymous caller provides valid foreign X-Workspace-Id: {wsForeign}
        // Traced production contract: WorkspaceResolutionMiddleware ignores X-Workspace-Id when user is anonymous (userId == null).
        // The endpoint is [WorkspaceNotRequired]. The signup succeeds without mutating wsForeign.
        var signupDto = new RegisterWorkspaceDto
        {
            Email = "sovereign.creator@signup-test.com",
            Password = "Password123!",
            FirstName = "Sovereign",
            Surname = "Creator",
            WorkspaceName = "Sovereign Global Co",
            PlanId = freePlan.Id
        };
        var reqWithAmbientHeader = CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: wsForeign, jsonBody: signupDto);
        var respWithAmbientHeader = await client.SendAsync(reqWithAmbientHeader);
        respWithAmbientHeader.StatusCode.ShouldBe(HttpStatusCode.OK);

        using var respDoc = JsonDocument.Parse(await respWithAmbientHeader.Content.ReadAsStringAsync());
        var newWsId = ReadGuidProperty(respDoc.RootElement, "workspaceId");
        newWsId.ShouldNotBe(wsForeign);

        // Explicit correct context verification: wsForeign and its sentinels are 100% UNMUTATED
        await WithUnitOfWorkAsync(async () =>
        {
            var newUser = await _userManager.FindByEmailAsync("sovereign.creator@signup-test.com");
            newUser.ShouldNotBeNull();

            // New user is Owner of new workspace
            var newMember = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == newWsId && m.UserId == newUser.Id);
            newMember.ShouldNotBeNull();
            newMember.Role.ShouldBe(WorkspaceRoles.Owner);

            // wsForeign owner is STILL ForeignOwner
            var owner = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsForeign && m.UserId == ForeignOwner);
            owner.ShouldNotBeNull();
            owner.Role.ShouldBe(WorkspaceRoles.Owner);

            // wsForeign member is STILL ForeignMember
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsForeign && m.UserId == ForeignMember);
            member.ShouldNotBeNull();
            member.Role.ShouldBe(WorkspaceRoles.Member);

            // New user is NOT a member of wsForeign
            var userInForeign = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsForeign && m.UserId == newUser.Id);
            userInForeign.ShouldBeNull();

            // Unrelated invitation B is still pending and untouched
            var invB = await _invitationRepository.GetAsync(invitationB.Id);
            invB.AcceptedUserId.ShouldBeNull();
            invB.AcceptedAt.ShouldBeNull();
            invB.IsUsable(DateTime.UtcNow).ShouldBeTrue();

            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var sub = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == wsForeign);
                sub.ShouldNotBeNull();
                sub.PlanId.ShouldBe(businessPlan.Id);

                var proj = await _projectRepository.FirstOrDefaultAsync(p => p.Id == sentinelProjId && p.WorkspaceId == wsForeign);
                proj.ShouldNotBeNull();
                proj.Title.ShouldBe("Foreign-Sentinel-Project");
            }
        });

        // 3. Subcase B: Malformed / empty GUID in X-Workspace-Id: 00000000-0000-0000-0000-000000000000
        // WorkspaceResolutionMiddleware intercepts with 400 BadRequest (Tracker:WorkspaceRequired)
        var malformedDto = new RegisterWorkspaceDto
        {
            Email = "malformed.header@signup-test.com",
            Password = "Password123!",
            WorkspaceName = "Malformed Header Co"
        };
        var reqMalformedHeader = CreateRequest(HttpMethod.Post, "/api/app/workspace/register", userId: null, workspaceId: Guid.Empty, jsonBody: malformedDto);
        var respMalformedHeader = await client.SendAsync(reqMalformedHeader);
        respMalformedHeader.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        (await respMalformedHeader.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);

        // Verify zero writes occurred for malformed request
        await WithUnitOfWorkAsync(async () =>
        {
            (await _userManager.FindByEmailAsync("malformed.header@signup-test.com")).ShouldBeNull();
            (await _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == "MALFORMED HEADER CO")).ShouldBeNull();
        });
    }
}
