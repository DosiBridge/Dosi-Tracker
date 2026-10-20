using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-DAT-02: Authenticated workspace-scoped activity CSV export regressions.
/// Tests genuine hosted TestServer HTTP endpoints, WorkspaceResolutionMiddleware,
/// aggregate workspace permissions (WorkspaceRolePermissionValueProvider),
/// server-enforced workload/date bounds, and formula injection neutralization.
/// </summary>
public class WorkspaceActivityExportHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid UserA = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid UserB = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid DualUser = Guid.Parse("4a701e62-0953-4dd3-910b-dc6cc93ccb04");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceActivityExportHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
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
    }

    private new static HttpRequestMessage CreateRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        Guid? workspaceId,
        object? jsonBody = null)
    {
        var req = WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("text/csv"));
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
                await _userManager.CreateAsync(new IdentityUser(UserA, "userA_exp", "userA_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ColleagueA, "colleagueA_exp", "colleagueA_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(UserB.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserB, "userB_exp", "userB_exp@acme.test"));
            }
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(DualUser, "dual_exp", "dual_exp@acme.test"));
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace A"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace B"), autoSave: true);

            // Workspace A members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, UserA, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);

            // Workspace B members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, UserB, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
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
    public async Task Authorized_Workspace_Owner_Exports_Csv_With_Headers_And_Data()
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
                    MouseClicks = 120,
                    KeyboardHits = 300,
                    Description = "Sprint planning task"
                };
                await _activityRepository.InsertAsync(activity, autoSave: true);
                actId = activity.Id;
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export", userId: UserA, workspaceId: wsA);
        var response = await client.SendAsync(req);

        response.StatusCode.ShouldBe(HttpStatusCode.OK);
        response.Content.Headers.ContentType?.ToString().ShouldBe("text/csv; charset=utf-8");
        response.Content.Headers.ContentDisposition?.DispositionType.ShouldBe("attachment");
        response.Content.Headers.ContentDisposition?.FileName.ShouldNotBeNull();
        response.Headers.GetValues("X-Total-Count").First().ShouldBe("1");
        response.Headers.GetValues("X-Exported-Count").First().ShouldBe("1");

        var csv = await response.Content.ReadAsStringAsync();
        csv.ShouldContain("ActivityId,UserId,ProjectId,ProjectTitle,StartedAtUtc,EndedAtUtc,DurationMinutes,DurationSeconds,Description,Productivity,IdleSeconds,MouseClicks,KeyboardHits");
        csv.ShouldContain(actId.ToString("D"));
        csv.ShouldContain(UserA.ToString("D"));
        csv.ShouldContain("Project Alpha");
        csv.ShouldContain("Sprint planning task");
        csv.ShouldContain("85");
        csv.ShouldContain("120");
        csv.ShouldContain("300");
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Attempting_Export_Is_Denied_With_403_NotWorkspaceMember()
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
        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export", userId: UserA, workspaceId: wsB);
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
            }
        });
    }

    [Fact]
    public async Task Same_User_With_Different_Roles_In_Workspaces_A_And_B_Exports_Appropriate_Data()
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

        // 1. DualUser in wsA (Owner role): sees Colleague A's activity
        var reqA = CreateRequest(HttpMethod.Get, "/api/app/activity/export", userId: DualUser, workspaceId: wsA);
        var respA = await client.SendAsync(reqA);
        respA.StatusCode.ShouldBe(HttpStatusCode.OK);
        var csvA = await respA.Content.ReadAsStringAsync();
        csvA.ShouldContain("Colleague A Activity in wsA");

        // 2. DualUser in wsB (Member role): sees ONLY own activity, User B's activity is excluded!
        var reqB = CreateRequest(HttpMethod.Get, "/api/app/activity/export", userId: DualUser, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var csvB = await respB.Content.ReadAsStringAsync();
        csvB.ShouldContain("DualUser Activity in wsB");
        csvB.ShouldNotContain("User B Activity in wsB");
    }

    [Fact]
    public async Task Regular_Member_Attempting_Other_User_Or_Foreign_Project_Filter_Is_Rejected()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ColleagueA is a regular member in wsA
        // 1. Attempts to filter by other user (UserA)
        var reqOtherUser = CreateRequest(
            HttpMethod.Get, $"/api/app/activity/export?userId={UserA}", userId: ColleagueA, workspaceId: wsA);
        var respOtherUser = await client.SendAsync(reqOtherUser);
        respOtherUser.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 2. Attempts to filter by project belonging to foreign workspace (projB in wsB)
        var reqForeignProject = CreateRequest(
            HttpMethod.Get, $"/api/app/activity/export?projectId={projB}", userId: ColleagueA, workspaceId: wsA);
        var respForeignProject = await client.SendAsync(reqForeignProject);
        (respForeignProject.StatusCode == HttpStatusCode.NotFound || respForeignProject.StatusCode == HttpStatusCode.Forbidden).ShouldBeTrue();
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired()
    {
        await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Missing header
        var reqMissing = new HttpRequestMessage(HttpMethod.Get, "/api/app/activity/export");
        reqMissing.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());
        reqMissing.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);

        // 2. Guid.Empty header
        var reqEmpty = new HttpRequestMessage(HttpMethod.Get, "/api/app/activity/export");
        reqEmpty.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());
        reqEmpty.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        reqEmpty.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());
        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Malformed_Or_Out_Of_Bounds_Dates_And_Oversize_Workload_Are_Rejected()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Malformed date range: To <= From
        var reqReversedDates = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export?from=2026-08-10T00:00:00Z&to=2026-08-01T00:00:00Z",
            userId: UserA, workspaceId: wsA);
        var respReversedDates = await client.SendAsync(reqReversedDates);
        respReversedDates.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyReversed = await respReversedDates.Content.ReadAsStringAsync();
        bodyReversed.ShouldContain(TrackerDomainErrorCodes.ReportRangeInvalid);

        // 2. Out of bounds date range: span > 366 days
        var reqExcessiveSpan = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export?from=2025-01-01T00:00:00Z&to=2026-06-01T00:00:00Z",
            userId: UserA, workspaceId: wsA);
        var respExcessiveSpan = await client.SendAsync(reqExcessiveSpan);
        respExcessiveSpan.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Oversize MaxResultCount > 10000 limit
        var reqOversizeLimit = CreateRequest(
            HttpMethod.Get,
            "/api/app/activity/export?maxResultCount=50000",
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
            "/api/app/activity/export?maxResultCount=2",
            userId: UserA, workspaceId: wsA);
        var respWorkloadExceeded = await client.SendAsync(reqWorkloadExceeded);
        respWorkloadExceeded.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyWorkload = await respWorkloadExceeded.Content.ReadAsStringAsync();
        bodyWorkload.ShouldContain("Tracker:ExportExceedsLimit");
    }

    [Fact]
    public async Task Csv_Escapes_Special_Characters_And_Neutralizes_Formula_Injection_While_Keeping_Unavailable_Values_Blank()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var baseTime = new DateTime(2026, 7, 20, 8, 0, 0, DateTimeKind.Utc);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                // Item 1: Comma, double quotes, newline
                var a1 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime, baseTime.AddMinutes(10))
                {
                    Description = "Feature \"A\", with comma and\nnewline"
                };

                // Item 2: Formulas: =SUM, +cmd, -10, @evil, leading whitespace, leading tab
                var a2 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(15), baseTime.AddMinutes(25))
                {
                    Description = "=SUM(1+1)"
                };
                var a3 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(30), baseTime.AddMinutes(40))
                {
                    Description = "+cmd|' /C calc'!A0"
                };
                var a4 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(45), baseTime.AddMinutes(55))
                {
                    Description = "   =2+2"
                };
                var a5 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(60), baseTime.AddMinutes(70))
                {
                    Description = "\t=1+1"
                };

                // Item 6: Unavailable metrics: Productivity = null, IdleSeconds = null, Description = null
                var a6 = new Activity(
                    _guidGenerator.Create(), wsA, UserA, projA, _guidGenerator.Create(),
                    baseTime.AddMinutes(75), baseTime.AddMinutes(85))
                {
                    Productivity = null,
                    IdleSeconds = null,
                    Description = null
                };

                await _activityRepository.InsertAsync(a1, autoSave: true);
                await _activityRepository.InsertAsync(a2, autoSave: true);
                await _activityRepository.InsertAsync(a3, autoSave: true);
                await _activityRepository.InsertAsync(a4, autoSave: true);
                await _activityRepository.InsertAsync(a5, autoSave: true);
                await _activityRepository.InsertAsync(a6, autoSave: true);
            }
        });

        var req = CreateRequest(HttpMethod.Get, "/api/app/activity/export", userId: UserA, workspaceId: wsA);
        var response = await client.SendAsync(req);
        response.StatusCode.ShouldBe(HttpStatusCode.OK);

        var csv = await response.Content.ReadAsStringAsync();

        // 1. Escaping: quotes doubled and wrapped
        csv.ShouldContain("\"Feature \"\"A\"\", with comma and\nnewline\"");

        // 2. Formula injection neutralized with single-quote prefix
        csv.ShouldContain("\"'=SUM(1+1)\"");
        csv.ShouldContain("\"'+cmd|' /C calc'!A0\"");
        csv.ShouldContain("\"'   =2+2\"");
        csv.ShouldContain("\"'\t=1+1\"");

        // 3. Unavailable values are empty string, not invented measured zeros
        // Looking for lines with empty productivity and idle fields: ,,
        var lines = csv.Split('\n').Select(l => l.TrimEnd('\r')).ToList();
        var emptyMetricRow = lines.FirstOrDefault(l => l.Contains(",,"));
        emptyMetricRow.ShouldNotBeNull();
    }
}
