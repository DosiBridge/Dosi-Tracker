using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Reporting;

/// <summary>
/// REQ-ISO-03 / REQ-ISO-05: Authenticated financial reporting authorization and workspace/query isolation over hosted HTTP.
/// Exercises genuine TestServer HTTP endpoints (GET /api/app/reporting/payroll), conventional controller query model binding,
/// WorkspaceResolutionMiddleware, restored aggregate ABP permissions (TrackerPermissions.Team.Manage),
/// scoped activity streaming, historical rate schedules, and cross-workspace query containment.
/// </summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspacePayrollReportingHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid DualUser = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid ForeignUserA = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid WithdrawnManagerA = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb05");

    private static readonly Guid OwnerB = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb06");
    private static readonly Guid WorkerB = Guid.Parse("7a701e62-0953-4dd3-910b-dc6cc93ccb07");

    private const decimal RateOwnerA = 120.00m;
    private const decimal RateDualUserA = 90.00m;
    private const decimal RateColleagueA = 60.00m;

    private const decimal RateOwnerB = 80.00m;
    private const decimal RateDualUserB = 50.00m;
    private const decimal RateWorkerB = 40.00m;

    private static readonly DateTime ReportFrom = new(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc);
    private static readonly DateTime ReportTo = new(2026, 10, 2, 0, 0, 0, DateTimeKind.Utc);

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspacePayrollReportingHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _projectMemberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _rateRepository = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
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

    private static string BuildPayrollUrl(DateTime from, DateTime to, Guid? projectId = null, Guid? userId = null)
    {
        var url = $"/api/app/reporting/payroll?from={Uri.EscapeDataString(from.ToString("o"))}&to={Uri.EscapeDataString(to.ToString("o"))}";
        if (projectId.HasValue)
        {
            url += $"&projectId={projectId.Value}";
        }
        if (userId.HasValue)
        {
            url += $"&userId={userId.Value}";
        }
        return url;
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA1, Guid ProjectB1)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(OwnerA.ToString()) == null)
            {
                var u = new IdentityUser(OwnerA, "owner_a_payroll", "owner_a_payroll@acme.test") { Name = "Owner", Surname = "Alpha" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                var u = new IdentityUser(ColleagueA, "colleague_a_payroll", "colleague_a_payroll@acme.test") { Name = "Colleague", Surname = "Alpha" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                var u = new IdentityUser(DualUser, "dual_user_payroll", "dual_user_payroll@acme.test") { Name = "Dual", Surname = "User" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(ForeignUserA.ToString()) == null)
            {
                var u = new IdentityUser(ForeignUserA, "foreign_a_payroll", "foreign_a_payroll@acme.test") { Name = "Foreign", Surname = "Alpha" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(WithdrawnManagerA.ToString()) == null)
            {
                var u = new IdentityUser(WithdrawnManagerA, "withdrawn_mgr_payroll", "withdrawn_mgr_payroll@acme.test") { Name = "Withdrawn", Surname = "Manager" };
                await _userManager.CreateAsync(u);
            }

            if (await _userManager.FindByIdAsync(OwnerB.ToString()) == null)
            {
                var u = new IdentityUser(OwnerB, "owner_b_payroll", "owner_b_payroll@acme.test") { Name = "Owner", Surname = "Beta" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(WorkerB.ToString()) == null)
            {
                var u = new IdentityUser(WorkerB, "worker_b_payroll", "worker_b_payroll@acme.test") { Name = "Worker", Surname = "Beta" };
                await _userManager.CreateAsync(u);
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace Alpha"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace Beta"), autoSave: true);

            // Workspace A memberships:
            // OwnerA is Owner, DualUser is Manager (privileged), ColleagueA is Member, ForeignUserA is Member, WithdrawnManagerA is Manager
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, OwnerA, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Manager, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ForeignUserA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, WithdrawnManagerA, WorkspaceRoles.Manager, DateTime.UtcNow), autoSave: true);

            // Workspace B memberships:
            // OwnerB is Owner, DualUser is regular Member (unprivileged in B, no Team.Manage), WorkerB is Member
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, OwnerB, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, WorkerB, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
        });

        Guid projA1 = Guid.Empty;
        Guid projB1 = Guid.Empty;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pA1 = new Project(_guidGenerator.Create(), wsA, "Project Alpha", color: "#112233");
                await _projectRepository.InsertAsync(pA1, autoSave: true);
                projA1 = pA1.Id;

                // Enroll project members
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, OwnerA, role: ProjectMemberRoles.Admin, hourlyRate: RateOwnerA), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, DualUser, role: ProjectMemberRoles.Manager, hourlyRate: RateDualUserA), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, ColleagueA, role: ProjectMemberRoles.Worker, hourlyRate: RateColleagueA), autoSave: true);

                // Add activities in Workspace A:
                // Block 1: ColleagueA from 09:00 to 10:30 (90 mins) at 60.00/h -> 90.00m
                var actA1 = new Activity(
                    _guidGenerator.Create(), wsA, ColleagueA, projA1, _guidGenerator.Create(),
                    ReportFrom.AddHours(9), ReportFrom.AddHours(10).AddMinutes(30))
                {
                    Productivity = 80,
                    MouseClicks = 100,
                    KeyboardHits = 200,
                    Description = "Colleague A Task"
                };
                await _activityRepository.InsertAsync(actA1, autoSave: true);

                // Block 2: DualUser from 11:00 to 12:00 (60 mins) at 90.00/h -> 90.00m
                var actA2 = new Activity(
                    _guidGenerator.Create(), wsA, DualUser, projA1, _guidGenerator.Create(),
                    ReportFrom.AddHours(11), ReportFrom.AddHours(12))
                {
                    Productivity = 90,
                    MouseClicks = 150,
                    KeyboardHits = 300,
                    Description = "DualUser A Task"
                };
                await _activityRepository.InsertAsync(actA2, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pB1 = new Project(_guidGenerator.Create(), wsB, "Project Beta", color: "#445566");
                await _projectRepository.InsertAsync(pB1, autoSave: true);
                projB1 = pB1.Id;

                // Enroll project members in B
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB1, OwnerB, role: ProjectMemberRoles.Admin, hourlyRate: RateOwnerB), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB1, DualUser, role: ProjectMemberRoles.Worker, hourlyRate: RateDualUserB), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB1, WorkerB, role: ProjectMemberRoles.Worker, hourlyRate: RateWorkerB), autoSave: true);

                // Add activities in Workspace B:
                // Block 1: WorkerB from 13:00 to 14:00 (60 mins) at 40.00/h -> 40.00m
                var actB1 = new Activity(
                    _guidGenerator.Create(), wsB, WorkerB, projB1, _guidGenerator.Create(),
                    ReportFrom.AddHours(13), ReportFrom.AddHours(14))
                {
                    Productivity = 75,
                    MouseClicks = 80,
                    KeyboardHits = 150,
                    Description = "Worker B Task"
                };
                await _activityRepository.InsertAsync(actB1, autoSave: true);

                // Block 2: DualUser from 14:00 to 14:30 (30 mins) at 50.00/h -> 25.00m
                var actB2 = new Activity(
                    _guidGenerator.Create(), wsB, DualUser, projB1, _guidGenerator.Create(),
                    ReportFrom.AddHours(14), ReportFrom.AddHours(14).AddMinutes(30))
                {
                    Productivity = 85,
                    MouseClicks = 90,
                    KeyboardHits = 180,
                    Description = "DualUser B Task"
                };
                await _activityRepository.InsertAsync(actB2, autoSave: true);
            }
        });

        return (wsA, wsB, projA1, projB1);
    }

    [Fact]
    public async Task Authorized_Owner_And_Manager_In_Workspace_A_Can_Read_Payroll_Over_Hosted_Http()
    {
        var (wsA, _, projA1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA calling GET /api/app/reporting/payroll in Workspace A
        var url = BuildPayrollUrl(ReportFrom, ReportTo);
        var reqOwner = CreateRequest(HttpMethod.Get, url, userId: OwnerA, workspaceId: wsA);
        var respOwner = await client.SendAsync(reqOwner);

        respOwner.StatusCode.ShouldBe(HttpStatusCode.OK);
        var jsonOwner = await respOwner.Content.ReadAsStringAsync();
        var resultOwner = JsonSerializer.Deserialize<PayrollReportDto>(jsonOwner, CamelCaseJsonOptions);

        resultOwner.ShouldNotBeNull();
        resultOwner.TotalMinutes.ShouldBe(150.0);
        resultOwner.TotalAmount.ShouldBe(180.00m);
        resultOwner.Rows.Count.ShouldBe(2);

        // Row 1: ColleagueA
        var rowColleague = resultOwner.Rows.Single(r => r.UserId == ColleagueA);
        rowColleague.UserName.ShouldBe("colleague_a_payroll");
        rowColleague.ProjectId.ShouldBe(projA1);
        rowColleague.ProjectTitle.ShouldBe("Project Alpha");
        rowColleague.TrackedMinutes.ShouldBe(90.0);
        rowColleague.HourlyRate.ShouldBe(60.00m);
        rowColleague.Amount.ShouldBe(90.00m);

        // Row 2: DualUser
        var rowDual = resultOwner.Rows.Single(r => r.UserId == DualUser);
        rowDual.UserName.ShouldBe("dual_user_payroll");
        rowDual.ProjectId.ShouldBe(projA1);
        rowDual.ProjectTitle.ShouldBe("Project Alpha");
        rowDual.TrackedMinutes.ShouldBe(60.0);
        rowDual.HourlyRate.ShouldBe(90.00m);
        rowDual.Amount.ShouldBe(90.00m);

        // Raw JSON inspection: strictly no Workspace B titles, usernames, or amounts present
        jsonOwner.ShouldNotContain("Project Beta");
        jsonOwner.ShouldNotContain("worker_b_payroll");
        jsonOwner.ShouldNotContain("65.00");
        jsonOwner.ShouldNotContain("40.00");
        jsonOwner.ShouldNotContain("25.00");

        // 2. DualUser is Manager in Workspace A (holds TrackerPermissions.Team.Manage)
        // Also permitted to read full team payroll in Workspace A
        var reqManager = CreateRequest(HttpMethod.Get, url, userId: DualUser, workspaceId: wsA);
        var respManager = await client.SendAsync(reqManager);
        respManager.StatusCode.ShouldBe(HttpStatusCode.OK);
        var resultManager = JsonSerializer.Deserialize<PayrollReportDto>(
            await respManager.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        resultManager.ShouldNotBeNull();
        resultManager.TotalMinutes.ShouldBe(150.0);
        resultManager.TotalAmount.ShouldBe(180.00m);
        resultManager.Rows.Count.ShouldBe(2);
    }

    [Fact]
    public async Task Authorized_Workspace_B_Owner_Can_Read_Separate_NonEmpty_Payroll_Over_Hosted_Http()
    {
        var (_, wsB, _, projB1) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Positive control: OwnerB calling GET /api/app/reporting/payroll in Workspace B
        var url = BuildPayrollUrl(ReportFrom, ReportTo);
        var req = CreateRequest(HttpMethod.Get, url, userId: OwnerB, workspaceId: wsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var json = await resp.Content.ReadAsStringAsync();
        var result = JsonSerializer.Deserialize<PayrollReportDto>(json, CamelCaseJsonOptions);

        result.ShouldNotBeNull();
        result.TotalMinutes.ShouldBe(90.0);
        result.TotalAmount.ShouldBe(65.00m);
        result.Rows.Count.ShouldBe(2);

        // Row 1: DualUser in B (30 mins @ 50.00/h -> 25.00m)
        var rowDual = result.Rows.Single(r => r.UserId == DualUser);
        rowDual.UserName.ShouldBe("dual_user_payroll");
        rowDual.ProjectId.ShouldBe(projB1);
        rowDual.ProjectTitle.ShouldBe("Project Beta");
        rowDual.TrackedMinutes.ShouldBe(30.0);
        rowDual.HourlyRate.ShouldBe(50.00m);
        rowDual.Amount.ShouldBe(25.00m);

        // Row 2: WorkerB in B (60 mins @ 40.00/h -> 40.00m)
        var rowWorker = result.Rows.Single(r => r.UserId == WorkerB);
        rowWorker.UserName.ShouldBe("worker_b_payroll");
        rowWorker.ProjectId.ShouldBe(projB1);
        rowWorker.ProjectTitle.ShouldBe("Project Beta");
        rowWorker.TrackedMinutes.ShouldBe(60.0);
        rowWorker.HourlyRate.ShouldBe(40.00m);
        rowWorker.Amount.ShouldBe(40.00m);

        // Raw JSON inspection: strictly no Workspace A data disclosed
        json.ShouldNotContain("Project Alpha");
        json.ShouldNotContain("colleague_a_payroll");
        json.ShouldNotContain("180.00");
        json.ShouldNotContain("90.00");
    }

    [Fact]
    public async Task Same_Account_Privileged_In_A_Cannot_Access_Payroll_In_Workspace_B_Where_Lower_Role()
    {
        var (wsA, wsB, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var url = BuildPayrollUrl(ReportFrom, ReportTo);

        // Step 1: DualUser queries Workspace A (where they are Manager -> privileged with Team.Manage)
        var reqA1 = CreateRequest(HttpMethod.Get, url, userId: DualUser, workspaceId: wsA);
        var respA1 = await client.SendAsync(reqA1);
        respA1.StatusCode.ShouldBe(HttpStatusCode.OK);

        // Step 2: DualUser switches to Workspace B (where they hold only regular Member role -> no Team.Manage)
        // Holding Manager rights in A MUST NOT bleed or grant Team.Manage in Workspace B
        var reqB = CreateRequest(HttpMethod.Get, url, userId: DualUser, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);

        respB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyB = await respB.Content.ReadAsStringAsync();
        bodyB.ShouldContain("010002");

        // Assert strictly zero Workspace B private financial, project, or member sentinels disclosed
        bodyB.ShouldNotContain("Project Beta");
        bodyB.ShouldNotContain("worker_b_payroll");
        bodyB.ShouldNotContain("65.00");
        bodyB.ShouldNotContain("40.00");
        bodyB.ShouldNotContain("25.00");

        // Step 3: DualUser switches back to Workspace A -> still authorized and receives valid payroll
        var reqA2 = CreateRequest(HttpMethod.Get, url, userId: DualUser, workspaceId: wsA);
        var respA2 = await client.SendAsync(reqA2);
        respA2.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docA2 = JsonSerializer.Deserialize<PayrollReportDto>(
            await respA2.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        docA2.ShouldNotBeNull();
        docA2.TotalAmount.ShouldBe(180.00m);
        docA2.TotalMinutes.ShouldBe(150.0);
    }

    [Fact]
    public async Task Regular_Member_In_Workspace_A_Is_Denied_Payroll_Access_With_403()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ColleagueA is a regular Member in Workspace A (tracked time exists, but lacks Team.Manage)
        var url = BuildPayrollUrl(ReportFrom, ReportTo);
        var req = CreateRequest(HttpMethod.Get, url, userId: ColleagueA, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("010002");

        // Must not disclose any financial amounts or team figures
        body.ShouldNotContain("180.00");
        body.ShouldNotContain("90.00");
        body.ShouldNotContain("Project Alpha");
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Spoofing_Target_Header_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (_, wsB, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ForeignUserA belongs strictly to Workspace A, not Workspace B
        // Attempts to read Workspace B payroll with spoofed X-Workspace-Id: wsB
        var url = BuildPayrollUrl(ReportFrom, ReportTo);
        var req = CreateRequest(HttpMethod.Get, url, userId: ForeignUserA, workspaceId: wsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Zero disclosure of target Workspace B sentinels
        body.ShouldNotContain("Project Beta");
        body.ShouldNotContain("worker_b_payroll");
        body.ShouldNotContain("65.00");
        body.ShouldNotContain("40.00");
    }

    [Fact]
    public async Task Authorized_Workspace_A_Query_With_Real_Foreign_Workspace_B_Project_Or_User_Returns_Empty_Scoped_Result()
    {
        var (wsA, _, _, projB1) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA in Workspace A queries with real Project Beta ID from Workspace B
        // EF Core workspace query scoping confines queries to Workspace A -> empty scoped result
        var urlProject = BuildPayrollUrl(ReportFrom, ReportTo, projectId: projB1);
        var reqProject = CreateRequest(HttpMethod.Get, urlProject, userId: OwnerA, workspaceId: wsA);
        var respProject = await client.SendAsync(reqProject);

        respProject.StatusCode.ShouldBe(HttpStatusCode.OK);
        var jsonProject = await respProject.Content.ReadAsStringAsync();
        var resultProject = JsonSerializer.Deserialize<PayrollReportDto>(jsonProject, CamelCaseJsonOptions);
        resultProject.ShouldNotBeNull();
        resultProject.Rows.ShouldBeEmpty();
        resultProject.TotalMinutes.ShouldBe(0);
        resultProject.TotalAmount.ShouldBe(0m);
        jsonProject.ShouldNotContain("Project Beta");
        jsonProject.ShouldNotContain("worker_b_payroll");
        jsonProject.ShouldNotContain("65.00");

        // 2. OwnerA in Workspace A queries with real WorkerB ID from Workspace B
        var urlUser = BuildPayrollUrl(ReportFrom, ReportTo, userId: WorkerB);
        var reqUser = CreateRequest(HttpMethod.Get, urlUser, userId: OwnerA, workspaceId: wsA);
        var respUser = await client.SendAsync(reqUser);

        respUser.StatusCode.ShouldBe(HttpStatusCode.OK);
        var jsonUser = await respUser.Content.ReadAsStringAsync();
        var resultUser = JsonSerializer.Deserialize<PayrollReportDto>(jsonUser, CamelCaseJsonOptions);
        resultUser.ShouldNotBeNull();
        resultUser.Rows.ShouldBeEmpty();
        resultUser.TotalMinutes.ShouldBe(0);
        resultUser.TotalAmount.ShouldBe(0m);
        jsonUser.ShouldNotContain("worker_b_payroll");
        jsonUser.ShouldNotContain("40.00");
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_Is_Rejected_With_Appropriate_Status()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var url = BuildPayrollUrl(ReportFrom, ReportTo);

        // 1. Missing workspace header
        // Intercepted by production WorkspaceRequiredFilter on conventional controller -> 400 BadRequest with WorkspaceRequired
        var reqMissing = CreateRequest(HttpMethod.Get, url, userId: OwnerA, workspaceId: null);
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
        bodyMissing.ShouldNotContain("180.00");

        // 2. Guid.Empty workspace header
        // Intercepted by production WorkspaceResolutionMiddleware -> 400 BadRequest with WorkspaceRequired
        var reqEmpty = new HttpRequestMessage(HttpMethod.Get, url);
        reqEmpty.Headers.Authorization = new AuthenticationHeaderValue("Bearer", OwnerA.ToString());
        reqEmpty.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        reqEmpty.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());

        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
        bodyEmpty.ShouldNotContain("180.00");
    }

    [Fact]
    public async Task Unauthenticated_Request_Is_Rejected_With_401_Unauthorized()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var url = BuildPayrollUrl(ReportFrom, ReportTo);

        // Request with valid Workspace header but no Authorization header
        var req = new HttpRequestMessage(HttpMethod.Get, url);
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        req.Headers.Add(WorkspaceConsts.HeaderName, wsA.ToString());

        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("010001");
        body.ShouldNotContain("180.00");
    }

    [Fact]
    public async Task Withdrawn_Member_From_Workspace_Is_Denied_On_Fresh_Payroll_Request()
    {
        var (wsA, wsB, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var url = BuildPayrollUrl(ReportFrom, ReportTo);

        // 1. WithdrawnManagerA starts as active Manager in Workspace A, then departs via production self-service leave endpoint
        var reqLeave = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/leave", userId: WithdrawnManagerA, workspaceId: null);
        var respLeave = await client.SendAsync(reqLeave);
        (respLeave.StatusCode == HttpStatusCode.OK || respLeave.StatusCode == HttpStatusCode.NoContent).ShouldBeTrue();

        // Verify persisted membership in SQLite is marked Removed
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == WithdrawnManagerA);
            member.ShouldNotBeNull();
            member.IsActive.ShouldBeFalse();
            member.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        });

        // Fresh payroll request from withdrawn user is denied with 403 Forbidden (Tracker:NotWorkspaceMember)
        var reqFresh = CreateRequest(HttpMethod.Get, url, userId: WithdrawnManagerA, workspaceId: wsA);
        var respFresh = await client.SendAsync(reqFresh);
        respFresh.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyFresh = await respFresh.Content.ReadAsStringAsync();
        bodyFresh.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        bodyFresh.ShouldNotContain("180.00");

        // 2. Unrelated Workspace B remains completely operational for its authorized owner
        var reqB = CreateRequest(HttpMethod.Get, url, userId: OwnerB, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docB = JsonSerializer.Deserialize<PayrollReportDto>(
            await respB.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        docB.ShouldNotBeNull();
        docB.TotalAmount.ShouldBe(65.00m);
    }

    [Fact]
    public async Task Target_Workspace_Persisted_Sentinels_Remain_Unchanged_Across_Battery_Of_Denied_Requests()
    {
        var (wsA, wsB, projA1, projB1) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var url = BuildPayrollUrl(ReportFrom, ReportTo);

        // Run battery of denied requests
        var req1 = CreateRequest(HttpMethod.Get, url, userId: ColleagueA, workspaceId: wsA); // regular member in A
        await client.SendAsync(req1);

        var req2 = CreateRequest(HttpMethod.Get, url, userId: DualUser, workspaceId: wsB); // regular member in B
        await client.SendAsync(req2);

        var req3 = CreateRequest(HttpMethod.Get, url, userId: ForeignUserA, workspaceId: wsB); // foreign caller
        await client.SendAsync(req3);

        // Assert explicit target-workspace persisted activity, rate, and membership sentinels remain 100% intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var activitiesA = await _activityRepository.GetListAsync(a => a.ProjectId == projA1);
                activitiesA.Count.ShouldBe(2);
                activitiesA.ShouldContain(a => a.UserId == ColleagueA && a.Productivity == 80);
                activitiesA.ShouldContain(a => a.UserId == DualUser && a.Productivity == 90);

                var membersA = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projA1);
                membersA.Count.ShouldBe(3);
                membersA.Single(m => m.UserId == ColleagueA).HourlyRate.ShouldBe(RateColleagueA);
                membersA.Single(m => m.UserId == DualUser).HourlyRate.ShouldBe(RateDualUserA);
                membersA.Single(m => m.UserId == OwnerA).HourlyRate.ShouldBe(RateOwnerA);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var activitiesB = await _activityRepository.GetListAsync(a => a.ProjectId == projB1);
                activitiesB.Count.ShouldBe(2);
                activitiesB.ShouldContain(a => a.UserId == WorkerB && a.Productivity == 75);
                activitiesB.ShouldContain(a => a.UserId == DualUser && a.Productivity == 85);

                var membersB = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB1);
                membersB.Count.ShouldBe(3);
                membersB.Single(m => m.UserId == WorkerB).HourlyRate.ShouldBe(RateWorkerB);
                membersB.Single(m => m.UserId == DualUser).HourlyRate.ShouldBe(RateDualUserB);
                membersB.Single(m => m.UserId == OwnerB).HourlyRate.ShouldBe(RateOwnerB);
            }
        });
    }
}
