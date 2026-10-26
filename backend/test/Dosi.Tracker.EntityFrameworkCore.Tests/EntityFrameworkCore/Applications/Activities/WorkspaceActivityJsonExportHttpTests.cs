using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Claims;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Linq;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-DAT-02: Authenticated workspace-scoped activity JSON export regressions.
/// Tests genuine hosted TestServer HTTP endpoints, WorkspaceResolutionMiddleware,
/// role-sensitive visibility, server-enforced workload/date bounds,
/// strict field allowlist, UTF-8/Bengali preservation, nullability handling,
/// and limit+1 concurrency overflow rejection at materialization boundary without partial JSON.
/// </summary>
public class WorkspaceActivityJsonExportHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid UserA = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid UserB = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid DualUser = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid NonMemberUser = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb05");
    private static readonly Guid WithdrawnUser = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb06");
    private static readonly Guid InactiveMemberUser = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb07");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IActivityAppService _activityAppService;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly ICurrentPrincipalAccessor _currentPrincipalAccessor;
    private readonly IWorkspaceAppService _workspaceAppService;
    private readonly WorkspaceMembershipManager _membershipManager;

    public WorkspaceActivityJsonExportHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _activityAppService = GetRequiredService<IActivityAppService>();
        _asyncExecuter = GetRequiredService<IAsyncQueryableExecuter>();
        _currentPrincipalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        _membershipManager = GetRequiredService<WorkspaceMembershipManager>();
    }

    private IDisposable SetCurrentUser(Guid userId, string? email = null)
    {
        var claims = new List<Claim>
        {
            new(AbpClaimTypes.UserId, userId.ToString()),
            new(ClaimTypes.NameIdentifier, userId.ToString())
        };
        if (email != null)
        {
            claims.Add(new Claim(AbpClaimTypes.Email, email));
        }

        return _currentPrincipalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "TestAuth")));
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        RegisterBarrierProxy(services);
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
        RegisterBarrierProxy(services);
    }

    private static void RegisterBarrierProxy(IServiceCollection services)
    {
        var existingExecuterDescriptor = services.FirstOrDefault(d => d.ServiceType == typeof(IAsyncQueryableExecuter));
        if (existingExecuterDescriptor != null)
        {
            services.Remove(existingExecuterDescriptor);
            services.AddTransient<IAsyncQueryableExecuter>(sp =>
            {
                IAsyncQueryableExecuter inner;
                if (existingExecuterDescriptor.ImplementationInstance != null)
                {
                    inner = (IAsyncQueryableExecuter)existingExecuterDescriptor.ImplementationInstance;
                }
                else if (existingExecuterDescriptor.ImplementationFactory != null)
                {
                    inner = (IAsyncQueryableExecuter)existingExecuterDescriptor.ImplementationFactory(sp);
                }
                else
                {
                    inner = (IAsyncQueryableExecuter)ActivatorUtilities.CreateInstance(sp, existingExecuterDescriptor.ImplementationType!);
                }
                var generator = new Castle.DynamicProxy.ProxyGenerator();
                return generator.CreateInterfaceProxyWithTarget<IAsyncQueryableExecuter>(inner, new BarrierInterceptor());
            });
        }
    }

    public class BarrierInterceptor : Castle.DynamicProxy.IInterceptor
    {
        public static Func<Task>? AfterCountHook { get; set; }

        public void Intercept(Castle.DynamicProxy.IInvocation invocation)
        {
            invocation.Proceed();

            if (invocation.Method.Name == nameof(IAsyncQueryableExecuter.CountAsync) && AfterCountHook != null && invocation.ReturnValue is Task<int> task)
            {
                var hook = AfterCountHook;
                AfterCountHook = null;
                invocation.ReturnValue = RunAfterCountAsync(task, hook);
            }
        }

        private static async Task<int> RunAfterCountAsync(Task<int> originalTask, Func<Task> hook)
        {
            var count = await originalTask;
            await hook();
            return count;
        }
    }

    private new static HttpRequestMessage CreateRequest(
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

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA, Guid ProjectB)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(UserA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserA, "userA_json_exp", "userA_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ColleagueA, "colleagueA_json_exp", "colleagueA_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(UserB.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserB, "userB_json_exp", "userB_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(DualUser, "dual_json_exp", "dual_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(NonMemberUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(NonMemberUser, "nonmember_json_exp", "nonmember_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(WithdrawnUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(WithdrawnUser, "withdrawn_json_exp", "withdrawn_json_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(InactiveMemberUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(InactiveMemberUser, "inactive_member_json_exp", "inactive_member_json_exp@acme.test"));
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace A"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace B"), autoSave: true);

            // Workspace A members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, UserA, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, WithdrawnUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            var mInactive = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveMemberUser, WorkspaceRoles.Member, DateTime.UtcNow);
            mInactive.Remove();
            await _memberRepository.InsertAsync(mInactive, autoSave: true);

            // Workspace B members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, UserB, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, WithdrawnUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
        });

        Guid projA = Guid.Empty;
        Guid projB = Guid.Empty;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pA = new Project(_guidGenerator.Create(), wsA, "Project Alpha", color: "#112233");
                await _projectRepository.InsertAsync(pA, autoSave: true);
                projA = pA.Id;
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pB = new Project(_guidGenerator.Create(), wsB, "Project Beta", color: "#445566");
                await _projectRepository.InsertAsync(pB, autoSave: true);
                projB = pB.Id;
            }
        });

        return (wsA, wsB, projA, projB);
    }

    [Fact]
    public async Task Authorized_Workspace_Owner_Exports_Json_With_Headers_Attachment_And_Valid_Document()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var startTime = new DateTime(2026, 7, 1, 10, 0, 0, DateTimeKind.Utc);
        var endTime = startTime.AddMinutes(15);

        Guid actId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var activity = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), startTime, endTime)
                {
                    Productivity = 85,
                    IdleSeconds = 45,
                    MouseClicks = 120,
                    KeyboardHits = 300,
                    Description = "Sprint planning task"
                };
                await _activityRepository.InsertAsync(activity, autoSave: true);
                actId = activity.Id;
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: UserA, workspaceId: wsA);
        var response = await client.SendAsync(req);

        response.StatusCode.ShouldBe(HttpStatusCode.OK);
        response.Content.Headers.ContentType?.ToString().ShouldBe("application/json; charset=utf-8");
        response.Content.Headers.ContentDisposition?.DispositionType.ShouldBe("attachment");
        response.Content.Headers.ContentDisposition?.FileName.ShouldNotBeNull();
        response.Content.Headers.ContentDisposition?.FileName.ShouldStartWith($"activities-{wsA:N}-");
        response.Content.Headers.ContentDisposition?.FileName.ShouldEndWith(".json");

        // Headers
        response.Headers.GetValues("X-Total-Count").First().ShouldBe("1");
        response.Headers.GetValues("X-Exported-Count").First().ShouldBe("1");
        response.Headers.CacheControl?.ToString().ShouldBe("no-store, private");
        response.Headers.Pragma.ToString().ShouldBe("no-cache");

        var jsonString = await response.Content.ReadAsStringAsync();
        var doc = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(jsonString, JsonExportHelper.JsonOptions);

        doc.ShouldNotBeNull();
        doc.Version.ShouldBe("1.1");
        doc.WorkspaceId.ShouldBe(wsA);
        doc.TotalCount.ShouldBe(1);
        doc.Activities.Count.ShouldBe(1);

        var item = doc.Activities[0];
        item.ActivityId.ShouldBe(actId);
        item.UserId.ShouldBe(UserA);
        item.ProjectId.ShouldBe(projA);
        item.ProjectTitle.ShouldBe("Project Alpha");
        item.StartedAtUtc.ShouldBe(startTime);
        item.EndedAtUtc.ShouldBe(endTime);
        item.DurationMinutes.ShouldBe(15.0);
        item.DurationSeconds.ShouldBe(900);
        item.Description.ShouldBe("Sprint planning task");
        item.Productivity.ShouldBe(85);
        item.IdleSeconds.ShouldBe(45);
        item.MouseClicks.ShouldBe(120);
        item.KeyboardHits.ShouldBe(300);
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Attempting_Json_Export_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (wsA, wsB, _, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Seed foreign sentinel activity in Workspace B
        Guid foreignActId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var activity = new Activity(
                    _guidGenerator.Create(), wsB, UserB, projB, _guidGenerator.Create(),
                    DateTime.UtcNow.AddMinutes(-30), DateTime.UtcNow)
                {
                    Description = "Confidential Sentinel B"
                };
                await _activityRepository.InsertAsync(activity, autoSave: true);
                foreignActId = activity.Id;
            }
        });

        // UserA belongs only to wsA; attempts to export wsB
        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: UserA, workspaceId: wsB);
        var response = await client.SendAsync(req);

        response.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await response.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Verify foreign sentinel is untouched
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var act = await _activityRepository.FindAsync(foreignActId);
                act.ShouldNotBeNull();
                act.Description.ShouldBe("Confidential Sentinel B");
            }
        });
    }

    [Fact]
    public async Task Same_User_With_Different_Roles_In_Workspaces_A_And_B_Exports_Appropriate_Json_Data()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var time = new DateTime(2026, 7, 10, 12, 0, 0, DateTimeKind.Utc);

        // Workspace A: DualUser is Owner; ColleagueA has activity
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var actColleagueA = new Activity(
                    _guidGenerator.Create(), wsA, ColleagueA, projA, _guidGenerator.Create(), time, time.AddMinutes(10))
                {
                    Description = "Colleague A Activity in wsA"
                };
                await _activityRepository.InsertAsync(actColleagueA, autoSave: true);
            }
        });

        // Workspace B: DualUser is Member; UserB has activity, DualUser has activity
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var actUserB = new Activity(
                    _guidGenerator.Create(), wsB, UserB, projB, _guidGenerator.Create(), time, time.AddMinutes(10))
                {
                    Description = "User B Activity in wsB"
                };
                var actDualInB = new Activity(
                    _guidGenerator.Create(), wsB, DualUser, projB, _guidGenerator.Create(), time.AddMinutes(15), time.AddMinutes(25))
                {
                    Description = "DualUser Activity in wsB"
                };
                await _activityRepository.InsertAsync(actUserB, autoSave: true);
                await _activityRepository.InsertAsync(actDualInB, autoSave: true);
            }
        });

        // 1. DualUser in wsA (Owner role): can view all members including Colleague A
        var reqA = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: DualUser, workspaceId: wsA);
        var respA = await client.SendAsync(reqA);
        respA.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docA = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(
            await respA.Content.ReadAsStringAsync(), JsonExportHelper.JsonOptions);
        docA.ShouldNotBeNull();
        docA.Activities.Any(a => a.Description == "Colleague A Activity in wsA").ShouldBeTrue();

        // 2. DualUser in wsB (Member role): can ONLY export own activity, User B's activity is excluded
        var reqB = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: DualUser, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docB = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(
            await respB.Content.ReadAsStringAsync(), JsonExportHelper.JsonOptions);
        docB.ShouldNotBeNull();
        docB.Activities.Count.ShouldBe(1);
        docB.Activities[0].Description.ShouldBe("DualUser Activity in wsB");
        docB.Activities.Any(a => a.Description == "User B Activity in wsB").ShouldBeFalse();
    }

    [Fact]
    public async Task Regular_Member_Attempting_Other_User_Or_Foreign_Project_Filter_Is_Rejected()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ColleagueA is a regular member in wsA
        // 1. Attempts to filter by other user (UserA) -> 403 Forbidden
        var reqOtherUser = CreateRequest(
            HttpMethod.Get, $"/api/app/activity/export/json?userId={UserA}", userId: ColleagueA, workspaceId: wsA);
        var respOtherUser = await client.SendAsync(reqOtherUser);
        respOtherUser.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 2. Attempts to filter by project belonging to foreign workspace (projB in wsB) -> 404 or 403
        var reqForeignProject = CreateRequest(
            HttpMethod.Get, $"/api/app/activity/export/json?projectId={projB}", userId: ColleagueA, workspaceId: wsA);
        var respForeignProject = await client.SendAsync(reqForeignProject);
        (respForeignProject.StatusCode == HttpStatusCode.NotFound || respForeignProject.StatusCode == HttpStatusCode.Forbidden).ShouldBeTrue();
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired()
    {
        await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Missing header
        var reqMissing = new HttpRequestMessage(HttpMethod.Get, "/api/app/activity/export/json");
        reqMissing.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());
        reqMissing.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);

        // 2. Guid.Empty header
        var reqEmpty = new HttpRequestMessage(HttpMethod.Get, "/api/app/activity/export/json");
        reqEmpty.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());
        reqEmpty.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        reqEmpty.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());
        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Never_Member_Caller_Is_Denied_Json_Export_With_403_NotWorkspaceMember()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // NonMemberUser was never added to wsA (zero membership row)
        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: NonMemberUser, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        body.ShouldNotContain("\"activities\":");
    }

    [Fact]
    public async Task Withdrawn_Member_After_Leave_Lifecycle_Is_Denied_Json_Export_In_A_While_Preserving_B_Authorization_And_Historical_Sentinels()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var time = new DateTime(2026, 7, 12, 10, 0, 0, DateTimeKind.Utc);
        Guid actAId = Guid.Empty;
        Guid actBId = Guid.Empty;

        // Seed real nonempty activity sentinels in both Workspace A and Workspace B for WithdrawnUser
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var actA = new Activity(
                    _guidGenerator.Create(), wsA, WithdrawnUser, projA, _guidGenerator.Create(), time, time.AddMinutes(15))
                {
                    Productivity = 80,
                    Description = "Withdrawn User Activity in wsA"
                };
                await _activityRepository.InsertAsync(actA, autoSave: true);
                actAId = actA.Id;
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var actB = new Activity(
                    _guidGenerator.Create(), wsB, WithdrawnUser, projB, _guidGenerator.Create(), time.AddMinutes(20), time.AddMinutes(35))
                {
                    Productivity = 88,
                    Description = "Withdrawn User Activity in wsB"
                };
                await _activityRepository.InsertAsync(actB, autoSave: true);
                actBId = actB.Id;
            }
        });

        // 1. First prove authorized JSON export for WithdrawnUser in A and B via actual production HTTP path
        var reqInitialA = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: WithdrawnUser, workspaceId: wsA);
        var respInitialA = await client.SendAsync(reqInitialA);
        respInitialA.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docInitialA = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(
            await respInitialA.Content.ReadAsStringAsync(), JsonExportHelper.JsonOptions);
        docInitialA.ShouldNotBeNull();
        docInitialA.TotalCount.ShouldBe(1);
        docInitialA.Activities[0].Description.ShouldBe("Withdrawn User Activity in wsA");

        var reqInitialB = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: WithdrawnUser, workspaceId: wsB);
        var respInitialB = await client.SendAsync(reqInitialB);
        respInitialB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docInitialB = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(
            await respInitialB.Content.ReadAsStringAsync(), JsonExportHelper.JsonOptions);
        docInitialB.ShouldNotBeNull();
        docInitialB.TotalCount.ShouldBe(1);
        docInitialB.Activities[0].Description.ShouldBe("Withdrawn User Activity in wsB");

        // 2. Withdraw A membership through production self-service leave / removal lifecycle
        // Note: WithdrawnUser is a non-last-owner target (role is Member, UserA is Owner)
        var reqLeave = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/leave", userId: WithdrawnUser, workspaceId: null);
        var respLeave = await client.SendAsync(reqLeave);
        (respLeave.StatusCode == HttpStatusCode.OK || respLeave.StatusCode == HttpStatusCode.NoContent).ShouldBeTrue();

        // Explicitly verify persisted membership becomes inactive/removed in A, while remaining active in B
        await WithUnitOfWorkAsync(async () =>
        {
            var memberA = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == WithdrawnUser);
            memberA.ShouldNotBeNull();
            memberA.IsActive.ShouldBeFalse();
            memberA.Status.ShouldBe(WorkspaceMemberStatus.Removed);

            var memberB = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsB && m.UserId == WithdrawnUser);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
            memberB.Status.ShouldBe(WorkspaceMemberStatus.Active);
        });

        // 3. Make NEW request on existing client/identity/header for A: require JSON export denial with 403 NotWorkspaceMember, no summary rows
        var reqAfterA = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: WithdrawnUser, workspaceId: wsA);
        var respAfterA = await client.SendAsync(reqAfterA);
        respAfterA.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyAfterA = await respAfterA.Content.ReadAsStringAsync();
        bodyAfterA.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        bodyAfterA.ShouldNotContain("\"activities\":");

        // B export must remain authorized with correct role-limited data
        var reqAfterB = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: WithdrawnUser, workspaceId: wsB);
        var respAfterB = await client.SendAsync(reqAfterB);
        respAfterB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docAfterB = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(
            await respAfterB.Content.ReadAsStringAsync(), JsonExportHelper.JsonOptions);
        docAfterB.ShouldNotBeNull();
        docAfterB.TotalCount.ShouldBe(1);
        docAfterB.Activities[0].Description.ShouldBe("Withdrawn User Activity in wsB");

        // 4. Verify historical A/B activity rows/project/membership sentinels remain intact in explicit target contexts
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var actA = await _activityRepository.FindAsync(actAId);
                actA.ShouldNotBeNull();
                actA.Description.ShouldBe("Withdrawn User Activity in wsA");

                var pA = await _projectRepository.FindAsync(projA);
                pA.ShouldNotBeNull();
                pA.Title.ShouldBe("Project Alpha");
            }

            using (_currentWorkspace.Change(wsB))
            {
                var actB = await _activityRepository.FindAsync(actBId);
                actB.ShouldNotBeNull();
                actB.Description.ShouldBe("Withdrawn User Activity in wsB");

                var pB = await _projectRepository.FindAsync(projB);
                pB.ShouldNotBeNull();
                pB.Title.ShouldBe("Project Beta");
            }
        });
    }

    [Fact]
    public async Task Owner_Removal_Lifecycle_Denies_Subsequent_Json_Export_While_Preserving_Historical_Activity()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var time = new DateTime(2026, 7, 13, 11, 0, 0, DateTimeKind.Utc);
        Guid colActId = Guid.Empty;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var act = new Activity(
                    _guidGenerator.Create(), wsA, ColleagueA, projA, _guidGenerator.Create(), time, time.AddMinutes(10))
                {
                    Description = "Colleague A Activity before removal"
                };
                await _activityRepository.InsertAsync(act, autoSave: true);
                colActId = act.Id;
            }
        });

        // 1. Initial positive control: ColleagueA can export own activity
        var reqInitial = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: ColleagueA, workspaceId: wsA);
        var respInitial = await client.SendAsync(reqInitial);
        respInitial.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Legitimate Owner UserA removes ColleagueA via production removal lifecycle
        await WithUnitOfWorkAsync(async () =>
        {
            using (SetCurrentUser(UserA, "userA_json_exp@acme.test"))
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var member = await _membershipManager.FindActiveAsync(wsA, ColleagueA);
                member.ShouldNotBeNull();
                await _membershipManager.RemoveAsync(member);
            }
        });

        // Verify persisted membership is now removed
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == ColleagueA);
            member.ShouldNotBeNull();
            member.IsActive.ShouldBeFalse();
            member.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        });

        // 3. New request by removed member is denied with 403 NotWorkspaceMember, no summary rows
        var reqAfter = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: ColleagueA, workspaceId: wsA);
        var respAfter = await client.SendAsync(reqAfter);
        respAfter.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyAfter = await respAfter.Content.ReadAsStringAsync();
        bodyAfter.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        bodyAfter.ShouldNotContain("\"activities\":");

        // 4. Verify historical activity row remains intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var act = await _activityRepository.FindAsync(colActId);
                act.ShouldNotBeNull();
                act.Description.ShouldBe("Colleague A Activity before removal");
            }
        });
    }

    [Fact]
    public async Task Persisted_Removed_Membership_In_Workspace_Is_Denied_Json_Export_With_403_NotWorkspaceMember()
    {
        var (wsA, wsB, _, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // InactiveMemberUser has a preexisting persisted row in wsA with Status == Removed
        await WithUnitOfWorkAsync(async () =>
        {
            var memberA = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == InactiveMemberUser);
            memberA.ShouldNotBeNull();
            memberA.IsActive.ShouldBeFalse();
            memberA.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        });

        // Seed foreign sentinel in wsB
        Guid foreignActId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var actB = new Activity(
                    _guidGenerator.Create(), wsB, UserB, projB, _guidGenerator.Create(),
                    DateTime.UtcNow.AddMinutes(-20), DateTime.UtcNow)
                {
                    Description = "Sentinel B for Inactive Member Test"
                };
                await _activityRepository.InsertAsync(actB, autoSave: true);
                foreignActId = actB.Id;
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: InactiveMemberUser, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        body.ShouldNotContain("\"activities\":");

        // Verify foreign sentinel untouched
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var actB = await _activityRepository.FindAsync(foreignActId);
                actB.ShouldNotBeNull();
                actB.Description.ShouldBe("Sentinel B for Inactive Member Test");
            }
        });
    }

    [Fact]
    public async Task Malformed_Or_Out_Of_Bounds_Dates_And_Oversize_Workload_Are_Rejected()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Malformed date range: To <= From
        var reqReversedDates = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export/json?from=2026-08-10T00:00:00Z&to=2026-08-01T00:00:00Z",
            userId: UserA, workspaceId: wsA);
        var respReversedDates = await client.SendAsync(reqReversedDates);
        respReversedDates.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyReversed = await respReversedDates.Content.ReadAsStringAsync();
        bodyReversed.ShouldContain(TrackerDomainErrorCodes.ReportRangeInvalid);

        // 2. Out of bounds date range: span > 366 days
        var reqExcessiveSpan = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export/json?from=2025-01-01T00:00:00Z&to=2026-06-01T00:00:00Z",
            userId: UserA, workspaceId: wsA);
        var respExcessiveSpan = await client.SendAsync(reqExcessiveSpan);
        respExcessiveSpan.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Oversize MaxResultCount > 10000 limit
        var reqOversizeLimit = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export/json?maxResultCount=50000",
            userId: UserA, workspaceId: wsA);
        var respOversizeLimit = await client.SendAsync(reqOversizeLimit);
        respOversizeLimit.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 4. Server-enforced workload limit: 3 activities seeded, requested limit is 2 -> rejected
        var baseTime = new DateTime(2026, 7, 15, 10, 0, 0, DateTimeKind.Utc);
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                for (int i = 0; i < 3; i++)
                {
                    var a = new Activity(
                        _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                        baseTime.AddMinutes(i * 20), baseTime.AddMinutes(i * 20 + 10))
                    {
                        Description = $"Workload item {i}"
                    };
                    await _activityRepository.InsertAsync(a, autoSave: true);
                }
            }
        });

        var reqWorkloadExceeded = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export/json?maxResultCount=2",
            userId: UserA, workspaceId: wsA);
        var respWorkloadExceeded = await client.SendAsync(reqWorkloadExceeded);
        respWorkloadExceeded.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyWorkload = await respWorkloadExceeded.Content.ReadAsStringAsync();
        bodyWorkload.ShouldContain("Tracker:ExportExceedsLimit");
    }

    [Fact]
    public async Task Json_Export_Preserves_Unavailable_Metrics_As_Null_Historical_Zeros_As_Zero_And_Escapes_NonAscii_Correctly()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var baseTime = new DateTime(2026, 7, 20, 8, 0, 0, DateTimeKind.Utc);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                // Item 1: Nullable metrics unmeasured -> preserved as null
                var a1 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime, baseTime.AddMinutes(10))
                {
                    Productivity = null,
                    IdleSeconds = null,
                    Description = null,
                    MouseClicks = 0,
                    KeyboardHits = 0
                };

                // Item 2: Bengali non-ASCII Unicode text, quotes, newlines, emoji
                var a2 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(15), baseTime.AddMinutes(25))
                {
                    Productivity = 90,
                    IdleSeconds = 0,
                    MouseClicks = 50,
                    KeyboardHits = 150,
                    Description = "বাংলা স্প্রিন্ট টাস্ক (Sprint Planning) \"উন্নয়ন\"\nনতুন লাইন 🚀"
                };

                // Item 3: input counts not measured (e.g. a Wayland session) -> null, not 0
                var a3 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(30), baseTime.AddMinutes(40))
                {
                    Description = "unmeasured input",
                    MouseClicks = null,
                    KeyboardHits = null
                };

                await _activityRepository.InsertAsync(a1, autoSave: true);
                await _activityRepository.InsertAsync(a2, autoSave: true);
                await _activityRepository.InsertAsync(a3, autoSave: true);
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: UserA, workspaceId: wsA);
        var response = await client.SendAsync(req);
        response.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonString = await response.Content.ReadAsStringAsync();

        // 1. Raw JSON inspection for explicit nulls
        jsonString.ShouldContain("\"productivity\":null");
        jsonString.ShouldContain("\"idleSeconds\":null");
        jsonString.ShouldContain("\"description\":null");

        // 2. Historical zeros preserved as 0
        jsonString.ShouldContain("\"mouseClicks\":0");
        jsonString.ShouldContain("\"keyboardHits\":0");
        jsonString.ShouldContain("\"mouseClicks\":null");
        jsonString.ShouldContain("\"keyboardHits\":null");

        // 3. Non-ASCII Bengali and emoji preservation
        jsonString.ShouldContain("বাংলা স্প্রিন্ট টাস্ক (Sprint Planning)");
        (jsonString.Contains("🚀") || jsonString.Contains("\\ud83d\\ude80") || jsonString.Contains("\\uD83D\\uDE80")).ShouldBeTrue();

        // 4. Strongly-typed deserialization roundtrip
        var doc = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(jsonString, JsonExportHelper.JsonOptions);
        doc.ShouldNotBeNull();
        doc.Activities.Count.ShouldBe(3);

        var unmeasuredItem = doc.Activities.Single(a => a.Description == "unmeasured input");
        unmeasuredItem.MouseClicks.ShouldBeNull();
        unmeasuredItem.KeyboardHits.ShouldBeNull();

        var nullItem = doc.Activities.First(a => a.Description == null);
        nullItem.Productivity.ShouldBeNull();
        nullItem.IdleSeconds.ShouldBeNull();
        nullItem.MouseClicks.ShouldBe(0);
        nullItem.KeyboardHits.ShouldBe(0);

        var bengaliItem = doc.Activities.First(a => a.Description != null && a.Description.Contains("বাংলা"));
        bengaliItem.Productivity.ShouldBe(90);
        bengaliItem.IdleSeconds.ShouldBe(0);
        bengaliItem.Description!.ShouldContain("বাংলা স্প্রিন্ট টাস্ক (Sprint Planning) \"উন্নয়ন\"\nনতুন লাইন 🚀");
    }

    [Fact]
    public async Task Json_Export_Field_Allowlist_Excludes_Sensitive_And_Internal_Fields()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var time = new DateTime(2026, 7, 22, 9, 0, 0, DateTimeKind.Utc);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var activity = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), time, time.AddMinutes(15))
                {
                    Productivity = 95,
                    IdleSeconds = 10,
                    MouseClicks = 100,
                    KeyboardHits = 250,
                    Description = "Sensitive inspection task",
                    ActiveWindowsJson = "[{\"title\":\"Confidential Banking Portal - Password Entry\",\"duration\":10}]",
                    RunningProgramsJson = "[\"chrome.exe\",\"keepass.exe\",\"secret_daemon.exe\"]",
                    TimelineJson = "[{\"minute\":1,\"clicks\":5}]"
                };
                await _activityRepository.InsertAsync(activity, autoSave: true);
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json", userId: UserA, workspaceId: wsA);
        var response = await client.SendAsync(req);
        response.StatusCode.ShouldBe(HttpStatusCode.OK);

        var rawJson = await response.Content.ReadAsStringAsync();

        // Allowlisted fields must be present
        rawJson.ShouldContain("\"activityId\":");
        rawJson.ShouldContain("\"userId\":");
        rawJson.ShouldContain("\"projectId\":");
        rawJson.ShouldContain("\"projectTitle\":");
        rawJson.ShouldContain("\"startedAtUtc\":");
        rawJson.ShouldContain("\"endedAtUtc\":");
        rawJson.ShouldContain("\"durationMinutes\":");
        rawJson.ShouldContain("\"durationSeconds\":");
        rawJson.ShouldContain("\"description\":");
        rawJson.ShouldContain("\"productivity\":");
        rawJson.ShouldContain("\"idleSeconds\":");
        rawJson.ShouldContain("\"mouseClicks\":");
        rawJson.ShouldContain("\"keyboardHits\":");

        // Sensitive, process, window, biometric, screenshot, and credential fields MUST be absent
        rawJson.ShouldNotContain("activeWindows");
        rawJson.ShouldNotContain("activeWindowsJson");
        rawJson.ShouldNotContain("Confidential Banking Portal");
        rawJson.ShouldNotContain("runningPrograms");
        rawJson.ShouldNotContain("runningProgramsJson");
        rawJson.ShouldNotContain("keepass.exe");
        rawJson.ShouldNotContain("secret_daemon.exe");
        rawJson.ShouldNotContain("timelineJson");
        rawJson.ShouldNotContain("screenshots");
        rawJson.ShouldNotContain("storageUrl");
        rawJson.ShouldNotContain("clientActivityId");
        rawJson.ShouldNotContain("hourlyRate");
        rawJson.ShouldNotContain("pay");
        rawJson.ShouldNotContain("token");
        rawJson.ShouldNotContain("password");
        rawJson.ShouldNotContain("secret");
    }

    [Fact]
    public async Task Production_Json_Export_Rejects_Limit_Plus_One_Candidate_Overflow_At_Materialization_Boundary_Without_Partial_Json()
    {
        BarrierInterceptor.AfterCountHook = null;
        try
        {
            var (wsA, wsB, projA, projB) = await SeedFixturesAsync();

            var t0 = new DateTime(2026, 8, 2, 10, 0, 0, DateTimeKind.Utc);
            Guid sentinelBId = Guid.Empty;
            await WithUnitOfWorkAsync(async () =>
            {
                using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
                {
                    var a1 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0, t0.AddMinutes(10))
                    {
                        Description = "Json Act 1"
                    };
                    var a2 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0.AddMinutes(15), t0.AddMinutes(25))
                    {
                        Description = "Json Act 2"
                    };
                    await _activityRepository.InsertAsync(a1, autoSave: true);
                    await _activityRepository.InsertAsync(a2, autoSave: true);
                }

                // Seed Workspace B sentinel to ensure isolation remains intact
                using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
                {
                    var sentinelB = new Activity(_guidGenerator.Create(), wsB, UserB, projB, _guidGenerator.Create(), t0, t0.AddMinutes(10))
                    {
                        Description = "Sentinel B in wsB for JSON"
                    };
                    await _activityRepository.InsertAsync(sentinelB, autoSave: true);
                    sentinelBId = sentinelB.Id;
                }
            });

            // 1. Direct production service invocation:
            // Set barrier hook to insert row 3 right after CountAsync finishes (preliminary count = 2 <= limit 2)
            BarrierInterceptor.AfterCountHook = async () =>
            {
                await WithUnitOfWorkAsync(async () =>
                {
                    using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
                    {
                        var a3 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0.AddMinutes(30), t0.AddMinutes(40))
                        {
                            Description = "Json Act 3 - inserted after count"
                        };
                        await _activityRepository.InsertAsync(a3, autoSave: true);
                    }
                });
            };

            await WithUnitOfWorkAsync(async () =>
            {
                using (_currentPrincipalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(new[]
                {
                    new Claim(AbpClaimTypes.UserId, UserA.ToString())
                }))))
                using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
                {
                    var ex = await Should.ThrowAsync<BusinessException>(async () =>
                    {
                        await _activityAppService.ExportJsonAsync(new ExportActivitiesInput { MaxResultCount = 2 });
                    });
                    ex.Code.ShouldBe(TrackerDomainErrorCodes.ExportExceedsLimit);
                }
            });

            // 2. Real hosted HTTP TestServer endpoint:
            // wsA now has 3 rows (a1, a2, a3). Request limit = 3.
            // Preliminary CountAsync passes (3 <= 3).
            // Barrier hook inserts row 4 right after CountAsync finishes.
            // Production materialization Take(3 + 1) catches 4 rows (> limit 3) and rejects with 400.
            BarrierInterceptor.AfterCountHook = async () =>
            {
                await WithUnitOfWorkAsync(async () =>
                {
                    using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
                    {
                        var a4 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0.AddMinutes(45), t0.AddMinutes(55))
                        {
                            Description = "Json Act 4 - inserted after count during HTTP"
                        };
                        await _activityRepository.InsertAsync(a4, autoSave: true);
                    }
                });
            };

            var client = CreateHttpClient();
            var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json?maxResultCount=3", userId: UserA, workspaceId: wsA);
            var resp = await client.SendAsync(req);

            // Corrected implementation rejects with 400 BadRequest
            resp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
            var body = await resp.Content.ReadAsStringAsync();
            body.ShouldContain("Tracker:ExportExceedsLimit");

            // Proof of no partial JSON emission: body is an error response, not a JSON summary document
            body.ShouldNotContain("\"activities\":");

            // Verify Workspace B sentinel was not touched
            await WithUnitOfWorkAsync(async () =>
            {
                using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
                {
                    var sentinel = await _activityRepository.FindAsync(sentinelBId);
                    sentinel.ShouldNotBeNull();
                    sentinel.Description.ShouldBe("Sentinel B in wsB for JSON");
                }
            });
        }
        finally
        {
            BarrierInterceptor.AfterCountHook = null;
        }
    }

    [Fact]
    public async Task Production_Json_Export_Exact_Limit_Succeeds_With_Coherent_Snapshot_Derived_Counts_And_Exact_Array_Items()
    {
        BarrierInterceptor.AfterCountHook = null;
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();

        var t0 = new DateTime(2026, 8, 3, 10, 0, 0, DateTimeKind.Utc);
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var a1 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0, t0.AddMinutes(10))
                {
                    Description = "Exact Json Item 1"
                };
                var a2 = new Activity(_guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(), t0.AddMinutes(15), t0.AddMinutes(25))
                {
                    Description = "Exact Json Item 2"
                };
                await _activityRepository.InsertAsync(a1, autoSave: true);
                await _activityRepository.InsertAsync(a2, autoSave: true);
            }
        });

        var client = CreateHttpClient();
        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export/json?maxResultCount=2", userId: UserA, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.OK);
        resp.Content.Headers.ContentType?.ToString().ShouldBe("application/json; charset=utf-8");

        // Derived snapshot counts must be exactly 2 and perfectly coherent
        resp.Headers.GetValues("X-Total-Count").First().ShouldBe("2");
        resp.Headers.GetValues("X-Exported-Count").First().ShouldBe("2");

        var jsonString = await resp.Content.ReadAsStringAsync();
        var doc = JsonSerializer.Deserialize<ActivityJsonExportDocumentDto>(jsonString, JsonExportHelper.JsonOptions);
        doc.ShouldNotBeNull();
        doc.TotalCount.ShouldBe(2);
        doc.Activities.Count.ShouldBe(2);
        doc.Activities[0].Description!.ShouldContain("Exact Json Item");
        doc.Activities[1].Description!.ShouldContain("Exact Json Item");
    }
}
