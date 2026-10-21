using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Teams;

/// <summary>
/// REQ-ISO-03 / REQ-ISO-05: Authenticated project team roster financial and identity masking over actual hosted HTTP.
/// Exercises genuine TestServer HTTP endpoints (GET /api/app/team/project-members/{projectId} and GET /api/app/team/members),
/// conventional controller model binding, WorkspaceResolutionMiddleware, SQLite membership resolution,
/// and aggregate ABP authorization (WorkspaceRolePermissionValueProvider + IPermissionChecker).
/// </summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspaceTeamRosterHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid ColleagueB = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid DualUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid ForeignUserA = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb05");
    private static readonly Guid WithdrawnUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb06");
    private static readonly Guid InactiveUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb07");

    private const decimal RateOwnerA = 120.00m;
    private const decimal RateColleagueA = 55.25m;
    private const decimal RateDualUserA = 75.50m;

    private const decimal RateColleagueB = 35.75m;
    private const decimal RateDualUserB = 45.00m;

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceTeamRosterHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _projectMemberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
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

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA1, Guid ProjectA2, Guid ProjectB1, Guid ProjectB2)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(OwnerA.ToString()) == null)
            {
                var u = new IdentityUser(OwnerA, "owner_a_roster", "owner_a_roster@acme.test") { Name = "Owner", Surname = "A" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                var u = new IdentityUser(ColleagueA, "colleague_a_roster", "colleague_a_roster@acme.test") { Name = "Colleague", Surname = "Alpha" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(ColleagueB.ToString()) == null)
            {
                var u = new IdentityUser(ColleagueB, "colleague_b_roster", "colleague_b_roster@acme.test") { Name = "Colleague", Surname = "Beta" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                var u = new IdentityUser(DualUser, "dual_user_roster", "dual_user_roster@acme.test") { Name = "Dual", Surname = "Member" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(ForeignUserA.ToString()) == null)
            {
                var u = new IdentityUser(ForeignUserA, "foreign_a_roster", "foreign_a_roster@acme.test") { Name = "Foreign", Surname = "A" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(WithdrawnUser.ToString()) == null)
            {
                var u = new IdentityUser(WithdrawnUser, "withdrawn_roster", "withdrawn_roster@acme.test") { Name = "Withdrawn", Surname = "User" };
                await _userManager.CreateAsync(u);
            }
            if (await _userManager.FindByIdAsync(InactiveUser.ToString()) == null)
            {
                var u = new IdentityUser(InactiveUser, "inactive_roster", "inactive_roster@acme.test") { Name = "Inactive", Surname = "User" };
                await _userManager.CreateAsync(u);
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace Alpha"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace Beta"), autoSave: true);

            // Workspace A memberships:
            // OwnerA is Owner, DualUser is Manager (privileged, has Team.Manage), ColleagueA is Member, ForeignUserA is Member, WithdrawnUser is Member
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, OwnerA, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Manager, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ForeignUserA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, WithdrawnUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);

            var mInactive = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveUser, WorkspaceRoles.Member, DateTime.UtcNow);
            mInactive.Remove();
            await _memberRepository.InsertAsync(mInactive, autoSave: true);

            // Workspace B memberships:
            // ColleagueB is Owner, DualUser is regular Member (unprivileged in B, no Team.Manage), OwnerA is Member in B
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, ColleagueB, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, OwnerA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
        });

        Guid projA1 = Guid.Empty;
        Guid projA2 = Guid.Empty;
        Guid projB1 = Guid.Empty;
        Guid projB2 = Guid.Empty;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pA1 = new Project(_guidGenerator.Create(), wsA, "Project A-1", color: "#112233");
                await _projectRepository.InsertAsync(pA1, autoSave: true);
                projA1 = pA1.Id;

                var pA2 = new Project(_guidGenerator.Create(), wsA, "Project A-2", color: "#223344");
                await _projectRepository.InsertAsync(pA2, autoSave: true);
                projA2 = pA2.Id;

                // Enroll project members in projA1 with distinctive nonzero rates
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, OwnerA, role: ProjectMemberRoles.Admin, hourlyRate: RateOwnerA), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, DualUser, role: ProjectMemberRoles.Manager, hourlyRate: RateDualUserA), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA1, ColleagueA, role: ProjectMemberRoles.Worker, hourlyRate: RateColleagueA), autoSave: true);

                // projA2 enrolls only OwnerA
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsA, projA2, OwnerA, role: ProjectMemberRoles.Admin, hourlyRate: RateOwnerA), autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pB1 = new Project(_guidGenerator.Create(), wsB, "Project B-1", color: "#445566");
                await _projectRepository.InsertAsync(pB1, autoSave: true);
                projB1 = pB1.Id;

                var pB2 = new Project(_guidGenerator.Create(), wsB, "Project B-2", color: "#556677");
                await _projectRepository.InsertAsync(pB2, autoSave: true);
                projB2 = pB2.Id;

                // Enroll in projB1: ColleagueB and DualUser (Worker)
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB1, ColleagueB, role: ProjectMemberRoles.Admin, hourlyRate: RateColleagueB), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB1, DualUser, role: ProjectMemberRoles.Worker, hourlyRate: RateDualUserB), autoSave: true);

                // Enroll in projB2: ONLY ColleagueB (DualUser is NOT enrolled)
                await _projectMemberRepository.InsertAsync(new ProjectMember(
                    _guidGenerator.Create(), wsB, projB2, ColleagueB, role: ProjectMemberRoles.Admin, hourlyRate: RateColleagueB), autoSave: true);
            }
        });

        return (wsA, wsB, projA1, projA2, projB1, projB2);
    }

    [Fact]
    public async Task Privileged_Workspace_A_Manager_Sees_Exact_Authorized_Private_Values_Over_Hosted_Http()
    {
        var (wsA, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // DualUser is Manager in Workspace A, holding TrackerPermissions.Team.Manage
        var req = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: DualUser, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var json = await resp.Content.ReadAsStringAsync();
        var result = JsonSerializer.Deserialize<ListResultDto<ProjectMemberDto>>(json, CamelCaseJsonOptions);

        result.ShouldNotBeNull();
        result.Items.Count.ShouldBe(3);

        // Positive control: exact nonzero private rates and real emails returned for permitted caller
        var ownerMember = result.Items.Single(m => m.UserId == OwnerA);
        ownerMember.HourlyRate.ShouldBe(RateOwnerA);
        ownerMember.Email.ShouldBe("owner_a_roster@acme.test");
        ownerMember.UserName.ShouldBe("owner_a_roster");

        var colleagueMember = result.Items.Single(m => m.UserId == ColleagueA);
        colleagueMember.HourlyRate.ShouldBe(RateColleagueA);
        colleagueMember.Email.ShouldBe("colleague_a_roster@acme.test");
        colleagueMember.UserName.ShouldBe("colleague_a_roster");

        var selfMember = result.Items.Single(m => m.UserId == DualUser);
        selfMember.HourlyRate.ShouldBe(RateDualUserA);
        selfMember.Email.ShouldBe("dual_user_roster@acme.test");
        selfMember.UserName.ShouldBe("dual_user_roster");

        // Also verify GET /api/app/team/members returns all active workspace members for privileged caller
        var reqMembers = CreateRequest(HttpMethod.Get, "/api/app/team/members", userId: DualUser, workspaceId: wsA);
        var respMembers = await client.SendAsync(reqMembers);
        respMembers.StatusCode.ShouldBe(HttpStatusCode.OK);
        var membersResult = JsonSerializer.Deserialize<ListResultDto<TeamMemberDto>>(
            await respMembers.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        membersResult.ShouldNotBeNull();
        membersResult.Items.Count.ShouldBeGreaterThanOrEqualTo(4); // OwnerA, DualUser, ColleagueA, ForeignUserA, WithdrawnUser
        membersResult.Items.ShouldContain(m => m.UserId == ColleagueA);
    }

    [Fact]
    public async Task Regular_Member_In_Workspace_B_Reads_Roster_With_Strict_Rate_And_Email_Masking()
    {
        var (_, wsB, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // DualUser is regular Member in Workspace B, lacking TrackerPermissions.Team.Manage
        var req = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB1}", userId: DualUser, workspaceId: wsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var json = await resp.Content.ReadAsStringAsync();
        var result = JsonSerializer.Deserialize<ListResultDto<ProjectMemberDto>>(json, CamelCaseJsonOptions);

        result.ShouldNotBeNull();
        result.Items.Count.ShouldBe(2);

        // ColleagueB: financial rate masked to 0m (legacy contract zero masking) and email masked to null
        var colleagueMember = result.Items.Single(m => m.UserId == ColleagueB);
        colleagueMember.HourlyRate.ShouldBe(0m);
        colleagueMember.Email.ShouldBeNull();
        colleagueMember.UserName.ShouldBe("colleague_b_roster");
        colleagueMember.Name.ShouldBe("Colleague");
        colleagueMember.Surname.ShouldBe("Beta");
        colleagueMember.Role.ShouldBe(ProjectMemberRoles.Admin);

        // Self in B: also masked under regular member roster read contract
        var selfMember = result.Items.Single(m => m.UserId == DualUser);
        selfMember.HourlyRate.ShouldBe(0m);
        selfMember.Email.ShouldBeNull();
        selfMember.UserName.ShouldBe("dual_user_roster");
        selfMember.Role.ShouldBe(ProjectMemberRoles.Worker);

        // Raw JSON inspection: strictly no Workspace A private rates or emails present
        json.ShouldNotContain(RateColleagueA.ToString());
        json.ShouldNotContain(RateDualUserA.ToString());
        json.ShouldNotContain(RateColleagueB.ToString());
        json.ShouldNotContain(RateDualUserB.ToString());
        json.ShouldNotContain("colleague_a_roster@acme.test");
        json.ShouldNotContain("colleague_b_roster@acme.test");

        // Verify GET /api/app/team/members in Workspace B: returns ONLY DualUser (colleagues scoped out)
        var reqMembers = CreateRequest(HttpMethod.Get, "/api/app/team/members", userId: DualUser, workspaceId: wsB);
        var respMembers = await client.SendAsync(reqMembers);
        respMembers.StatusCode.ShouldBe(HttpStatusCode.OK);
        var membersResult = JsonSerializer.Deserialize<ListResultDto<TeamMemberDto>>(
            await respMembers.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        membersResult.ShouldNotBeNull();
        var selfOnly = membersResult.Items.ShouldHaveSingleItem();
        selfOnly.UserId.ShouldBe(DualUser);
        selfOnly.UserName.ShouldBe("dual_user_roster");
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Spoofing_Target_Header_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (_, wsB, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ForeignUserA belongs ONLY to Workspace A. Attempts to access Workspace B roster with spoofed header
        var req = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB1}", userId: ForeignUserA, workspaceId: wsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Assert no sensitive sentinel values in denied body
        body.ShouldNotContain(RateColleagueB.ToString());
        body.ShouldNotContain(RateDualUserB.ToString());
        body.ShouldNotContain("colleague_b_roster");
        body.ShouldNotContain("colleague_b_roster@acme.test");

        // Seeded foreign state, rates, and memberships preserved with explicit target context
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var membersB = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB1);
                membersB.Count.ShouldBe(2);
                membersB.Single(m => m.UserId == ColleagueB).HourlyRate.ShouldBe(RateColleagueB);
                membersB.Single(m => m.UserId == DualUser).HourlyRate.ShouldBe(RateDualUserB);
            }
        });
    }

    [Fact]
    public async Task Workspace_A_Header_With_Real_Foreign_Project_B_Id_Is_Denied_Or_Not_Found()
    {
        var (wsA, wsB, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ColleagueA is Member in Workspace A. Passes X-Workspace-Id: wsA but queries real Project B-1 ID
        var req = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB1}", userId: ColleagueA, workspaceId: wsA);
        var resp = await client.SendAsync(req);

        // Denied (403 Forbidden) by project membership guard since ColleagueA is not enrolled in projB1 in wsA
        (resp.StatusCode == HttpStatusCode.Forbidden || resp.StatusCode == HttpStatusCode.NotFound).ShouldBeTrue();

        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldNotContain(RateColleagueB.ToString());
        body.ShouldNotContain(RateDualUserB.ToString());
        body.ShouldNotContain("colleague_b_roster@acme.test");

        // Verify Workspace B sentinels intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var membersB = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB1);
                membersB.Count.ShouldBe(2);
                membersB.Single(m => m.UserId == ColleagueB).HourlyRate.ShouldBe(RateColleagueB);
            }
        });
    }

    [Fact]
    public async Task Regular_Member_Lacking_Target_Project_Membership_Is_Denied_With_403()
    {
        var (_, wsB, _, _, _, projB2) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // In Workspace B, DualUser is a member of wsB, but is NOT enrolled in projB2 (only ColleagueB is enrolled)
        var req = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB2}", userId: DualUser, workspaceId: wsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("010002");
        body.ShouldNotContain(RateColleagueB.ToString());
        body.ShouldNotContain("colleague_b_roster@acme.test");

        // Verify projB2 membership and rate remain completely untouched
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var membersB2 = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB2);
                membersB2.Count.ShouldBe(1);
                membersB2.Single().HourlyRate.ShouldBe(RateColleagueB);
            }
        });
    }

    [Fact]
    public async Task Inactive_Or_Withdrawn_Workspace_Member_Is_Denied_On_Fresh_Request_With_403()
    {
        var (wsA, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Withdrawn member lifecycle:
        // WithdrawnUser starts as active member of wsA, then leaves via production self-service leave endpoint
        var reqLeave = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/leave", userId: WithdrawnUser, workspaceId: null);
        var respLeave = await client.SendAsync(reqLeave);
        (respLeave.StatusCode == HttpStatusCode.OK || respLeave.StatusCode == HttpStatusCode.NoContent).ShouldBeTrue();

        // Verify persisted membership is now removed
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == wsA && m.UserId == WithdrawnUser);
            member.ShouldNotBeNull();
            member.IsActive.ShouldBeFalse();
            member.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        });

        // Fresh request from withdrawn user is denied with 403 NotWorkspaceMember
        var reqWithdrawn = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: WithdrawnUser, workspaceId: wsA);
        var respWithdrawn = await client.SendAsync(reqWithdrawn);
        respWithdrawn.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyWithdrawn = await respWithdrawn.Content.ReadAsStringAsync();
        bodyWithdrawn.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        bodyWithdrawn.ShouldNotContain(RateOwnerA.ToString());
        bodyWithdrawn.ShouldNotContain(RateColleagueA.ToString());
        bodyWithdrawn.ShouldNotContain("colleague_a_roster@acme.test");

        // 2. Pre-existing persisted inactive/removed user
        var reqInactive = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: InactiveUser, workspaceId: wsA);
        var respInactive = await client.SendAsync(reqInactive);
        respInactive.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyInactive = await respInactive.Content.ReadAsStringAsync();
        bodyInactive.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        bodyInactive.ShouldNotContain(RateOwnerA.ToString());
        bodyInactive.ShouldNotContain("owner_a_roster@acme.test");

        // Historical project members and rates in wsA remain intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var membersA = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projA1);
                membersA.Count.ShouldBe(3);
                membersA.Single(m => m.UserId == OwnerA).HourlyRate.ShouldBe(RateOwnerA);
                membersA.Single(m => m.UserId == ColleagueA).HourlyRate.ShouldBe(RateColleagueA);
            }
        });
    }

    [Fact]
    public async Task Same_User_A_Plus_B_Switching_Preserves_Privileged_A_And_Masked_B_Rosters_Sequentially()
    {
        var (wsA, wsB, projA1, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. DualUser queries Workspace A (where they are Manager -> privileged)
        var reqA1 = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: DualUser, workspaceId: wsA);
        var respA1 = await client.SendAsync(reqA1);
        respA1.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docA1 = JsonSerializer.Deserialize<ListResultDto<ProjectMemberDto>>(
            await respA1.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        docA1.ShouldNotBeNull();
        docA1.Items.Single(m => m.UserId == ColleagueA).HourlyRate.ShouldBe(RateColleagueA);
        docA1.Items.Single(m => m.UserId == ColleagueA).Email.ShouldBe("colleague_a_roster@acme.test");

        // 2. DualUser switches to Workspace B (where they are Member -> masked)
        var reqB = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB1}", userId: DualUser, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docB = JsonSerializer.Deserialize<ListResultDto<ProjectMemberDto>>(
            await respB.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        docB.ShouldNotBeNull();
        docB.Items.Single(m => m.UserId == ColleagueB).HourlyRate.ShouldBe(0m);
        docB.Items.Single(m => m.UserId == ColleagueB).Email.ShouldBeNull();

        // 3. DualUser switches back to Workspace A -> still privileged and unmasked
        var reqA2 = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: DualUser, workspaceId: wsA);
        var respA2 = await client.SendAsync(reqA2);
        respA2.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docA2 = JsonSerializer.Deserialize<ListResultDto<ProjectMemberDto>>(
            await respA2.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        docA2.ShouldNotBeNull();
        docA2.Items.Single(m => m.UserId == ColleagueA).HourlyRate.ShouldBe(RateColleagueA);
        docA2.Items.Single(m => m.UserId == ColleagueA).Email.ShouldBe("colleague_a_roster@acme.test");
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired()
    {
        var (_, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Missing header
        var reqMissing = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: OwnerA, workspaceId: null);
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
        bodyMissing.ShouldNotContain(RateOwnerA.ToString());

        // 2. Guid.Empty header
        var reqEmpty = new HttpRequestMessage(HttpMethod.Get, $"/api/app/team/project-members/{projA1}");
        reqEmpty.Headers.Authorization = new AuthenticationHeaderValue("Bearer", OwnerA.ToString());
        reqEmpty.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        reqEmpty.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());
        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
        bodyEmpty.ShouldNotContain(RateOwnerA.ToString());
    }

    [Fact]
    public async Task Sentinels_Preservation_Across_Denied_And_Masked_Requests()
    {
        var (wsA, wsB, projA1, projA2, projB1, projB2) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Run battery of denied and masked calls
        var req1 = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB1}", userId: ForeignUserA, workspaceId: wsB);
        await client.SendAsync(req1);

        var req2 = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projB2}", userId: DualUser, workspaceId: wsB);
        await client.SendAsync(req2);

        var req3 = CreateRequest(HttpMethod.Get, $"/api/app/team/project-members/{projA1}", userId: InactiveUser, workspaceId: wsA);
        await client.SendAsync(req3);

        // Verify sentinels row-for-row in explicit target contexts
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pA1 = await _projectRepository.FindAsync(projA1);
                pA1.ShouldNotBeNull();
                pA1.Title.ShouldBe("Project A-1");

                var pA2 = await _projectRepository.FindAsync(projA2);
                pA2.ShouldNotBeNull();
                pA2.Title.ShouldBe("Project A-2");

                var membersA1 = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projA1);
                membersA1.Count.ShouldBe(3);
                membersA1.Single(m => m.UserId == OwnerA).HourlyRate.ShouldBe(RateOwnerA);
                membersA1.Single(m => m.UserId == DualUser).HourlyRate.ShouldBe(RateDualUserA);
                membersA1.Single(m => m.UserId == ColleagueA).HourlyRate.ShouldBe(RateColleagueA);

                var membersA2 = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projA2);
                membersA2.Count.ShouldBe(1);
                membersA2.Single(m => m.UserId == OwnerA).HourlyRate.ShouldBe(RateOwnerA);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pB1 = await _projectRepository.FindAsync(projB1);
                pB1.ShouldNotBeNull();
                pB1.Title.ShouldBe("Project B-1");

                var pB2 = await _projectRepository.FindAsync(projB2);
                pB2.ShouldNotBeNull();
                pB2.Title.ShouldBe("Project B-2");

                var membersB1 = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB1);
                membersB1.Count.ShouldBe(2);
                membersB1.Single(m => m.UserId == ColleagueB).HourlyRate.ShouldBe(RateColleagueB);
                membersB1.Single(m => m.UserId == DualUser).HourlyRate.ShouldBe(RateDualUserB);

                var membersB2 = await _projectMemberRepository.GetListAsync(m => m.ProjectId == projB2);
                membersB2.Count.ShouldBe(1);
                membersB2.Single(m => m.UserId == ColleagueB).HourlyRate.ShouldBe(RateColleagueB);
            }
        });
    }

    [Fact]
    public async Task Authorized_Owner_Can_Update_Project_Member_Rate_And_Role_With_Persistence()
    {
        var (wsA, wsB, projA1, projA2, projB1, projB2) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Positive mutation control: OwnerA in wsA updates ColleagueA's role to Manager and hourly rate to 68.50m
        var updateDto = new UpdateProjectMemberDto
        {
            Role = ProjectMemberRoles.Manager,
            HourlyRate = 68.50m
        };

        var req = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: OwnerA,
            workspaceId: wsA,
            jsonBody: updateDto);

        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var json = await resp.Content.ReadAsStringAsync();
        var result = JsonSerializer.Deserialize<ProjectMemberDto>(json, CamelCaseJsonOptions);
        result.ShouldNotBeNull();
        result.ProjectId.ShouldBe(projA1);
        result.UserId.ShouldBe(ColleagueA);
        result.Role.ShouldBe(ProjectMemberRoles.Manager);
        result.HourlyRate.ShouldBe(68.50m);
        result.UserName.ShouldBe("colleague_a_roster");
        result.Email.ShouldBe("colleague_a_roster@acme.test");

        // Verify state persisted accurately in SQLite database
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var target = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == ColleagueA);
                target.ShouldNotBeNull();
                target.Role.ShouldBe(ProjectMemberRoles.Manager);
                target.HourlyRate.ShouldBe(68.50m);

                // Unrelated Workspace A rows remain intact
                var ownerInA1 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == OwnerA);
                ownerInA1.ShouldNotBeNull();
                ownerInA1.HourlyRate.ShouldBe(RateOwnerA);
                ownerInA1.Role.ShouldBe(ProjectMemberRoles.Admin);

                var dualInA1 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == DualUser);
                dualInA1.ShouldNotBeNull();
                dualInA1.HourlyRate.ShouldBe(RateDualUserA);
                dualInA1.Role.ShouldBe(ProjectMemberRoles.Manager);

                var ownerInA2 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA2 && m.UserId == OwnerA);
                ownerInA2.ShouldNotBeNull();
                ownerInA2.HourlyRate.ShouldBe(RateOwnerA);
            }

            // Target Workspace B rows remain completely unaffected
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var colleagueB1 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == ColleagueB);
                colleagueB1.ShouldNotBeNull();
                colleagueB1.HourlyRate.ShouldBe(RateColleagueB);
                colleagueB1.Role.ShouldBe(ProjectMemberRoles.Admin);

                var dualB1 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == DualUser);
                dualB1.ShouldNotBeNull();
                dualB1.HourlyRate.ShouldBe(RateDualUserB);
                dualB1.Role.ShouldBe(ProjectMemberRoles.Worker);

                var colleagueB2 = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB2 && m.UserId == ColleagueB);
                colleagueB2.ShouldNotBeNull();
                colleagueB2.HourlyRate.ShouldBe(RateColleagueB);
            }
        });
    }

    [Fact]
    public async Task Lower_Role_Cannot_Change_Rates_Or_Elevate_Role()
    {
        var (wsA, wsB, projA1, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. DualUser is Manager in Workspace A (has Team.Manage aggregate permission)
        // Attempting to change ColleagueA's rate is rejected by domain rule with 403 RoleNotAllowed
        var reqRateChange = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: DualUser,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = 99.00m });
        var respRateChange = await client.SendAsync(reqRateChange);
        respRateChange.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyRateChange = await respRateChange.Content.ReadAsStringAsync();
        bodyRateChange.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // Attempting to elevate ColleagueA to Manager role without owner rights is rejected with 403 RoleNotAllowed
        var reqElevateManager = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: DualUser,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = RateColleagueA });
        var respElevateManager = await client.SendAsync(reqElevateManager);
        respElevateManager.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyElevateManager = await respElevateManager.Content.ReadAsStringAsync();
        bodyElevateManager.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // Attempting to elevate ColleagueA to Admin role is rejected with 403 RoleNotAllowed
        var reqElevateAdmin = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: DualUser,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Admin, HourlyRate = RateColleagueA });
        var respElevateAdmin = await client.SendAsync(reqElevateAdmin);
        respElevateAdmin.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyElevateAdmin = await respElevateAdmin.Content.ReadAsStringAsync();
        bodyElevateAdmin.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // 2. ColleagueA is regular Member in Workspace A (lacks Team.Manage)
        // Blocked at aggregate MVC authorization layer with 403 (policy not granted)
        var reqMemberInA = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={DualUser}",
            userId: ColleagueA,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = 20.00m });
        var respMemberInA = await client.SendAsync(reqMemberInA);
        respMemberInA.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyMemberInA = await respMemberInA.Content.ReadAsStringAsync();
        bodyMemberInA.ShouldContain("010002");

        // 3. DualUser is regular Member in Workspace B (lacks Team.Manage in B)
        // Blocked at aggregate MVC authorization layer with 403 (policy not granted)
        var reqMemberInB = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projB1}&userId={ColleagueB}",
            userId: DualUser,
            workspaceId: wsB,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = 10.00m });
        var respMemberInB = await client.SendAsync(reqMemberInB);
        respMemberInB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyMemberInB = await respMemberInB.Content.ReadAsStringAsync();
        bodyMemberInB.ShouldContain("010002");

        // Verify sentinels row-for-row in both workspaces remain untouched
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var memberColleagueA = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == ColleagueA);
                memberColleagueA.ShouldNotBeNull();
                memberColleagueA.HourlyRate.ShouldBe(RateColleagueA);
                memberColleagueA.Role.ShouldBe(ProjectMemberRoles.Worker);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var memberColleagueB = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == ColleagueB);
                memberColleagueB.ShouldNotBeNull();
                memberColleagueB.HourlyRate.ShouldBe(RateColleagueB);
                memberColleagueB.Role.ShouldBe(ProjectMemberRoles.Admin);
            }
        });
    }

    [Fact]
    public async Task Same_Account_Switching_Never_Carries_Workspace_A_Owner_Rights_To_Workspace_B()
    {
        var (wsA, wsB, projA1, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA is Owner in Workspace A -> legitimate mutation succeeds
        var reqA1 = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: OwnerA,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = 62.00m });
        var respA1 = await client.SendAsync(reqA1);
        respA1.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. OwnerA switches to Workspace B (where they hold only Member role, unprivileged)
        // Owner rights from Workspace A MUST NOT leak or grant Team.Manage in Workspace B
        var reqB = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projB1}&userId={DualUser}",
            userId: OwnerA,
            workspaceId: wsB,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = 99.00m });
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyB = await respB.Content.ReadAsStringAsync();
        bodyB.ShouldContain("010002");

        // 3. DualUser switches from privileged Manager in A to unprivileged Member in B
        var reqDualInB = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projB1}&userId={ColleagueB}",
            userId: DualUser,
            workspaceId: wsB,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = 12.00m });
        var respDualInB = await client.SendAsync(reqDualInB);
        respDualInB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyDualInB = await respDualInB.Content.ReadAsStringAsync();
        bodyDualInB.ShouldContain("010002");

        // 4. OwnerA switches back to Workspace A -> still authorized Owner
        var reqA2 = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: OwnerA,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = 59.00m });
        var respA2 = await client.SendAsync(reqA2);
        respA2.StatusCode.ShouldBe(HttpStatusCode.OK);

        // Verify Workspace B database sentinels remained 100% unmutated throughout
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var dualInB = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == DualUser);
                dualInB.ShouldNotBeNull();
                dualInB.HourlyRate.ShouldBe(RateDualUserB);
                dualInB.Role.ShouldBe(ProjectMemberRoles.Worker);

                var colleagueInB = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == ColleagueB);
                colleagueInB.ShouldNotBeNull();
                colleagueInB.HourlyRate.ShouldBe(RateColleagueB);
                colleagueInB.Role.ShouldBe(ProjectMemberRoles.Admin);
            }

            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var colleagueInA = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == ColleagueA);
                colleagueInA.ShouldNotBeNull();
                colleagueInA.HourlyRate.ShouldBe(59.00m);
                colleagueInA.Role.ShouldBe(ProjectMemberRoles.Worker);
            }
        });
    }

    [Fact]
    public async Task Cannot_Modify_Own_Role_Or_Rate_Even_As_Privileged_Owner_Or_Manager()
    {
        var (wsA, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA (Workspace Owner) attempting to modify self rate
        var reqOwnerSelf = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={OwnerA}",
            userId: OwnerA,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Admin, HourlyRate = 250.00m });
        var respOwnerSelf = await client.SendAsync(reqOwnerSelf);
        respOwnerSelf.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyOwnerSelf = await respOwnerSelf.Content.ReadAsStringAsync();
        bodyOwnerSelf.ShouldContain(TrackerDomainErrorCodes.CannotModifySelf);

        // 2. DualUser (Workspace Manager) attempting to modify self role
        var reqManagerSelf = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={DualUser}",
            userId: DualUser,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = RateDualUserA });
        var respManagerSelf = await client.SendAsync(reqManagerSelf);
        respManagerSelf.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyManagerSelf = await respManagerSelf.Content.ReadAsStringAsync();
        bodyManagerSelf.ShouldContain(TrackerDomainErrorCodes.CannotModifySelf);

        // Verify database sentinels remain unchanged
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var owner = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == OwnerA);
                owner.ShouldNotBeNull();
                owner.HourlyRate.ShouldBe(RateOwnerA);
                owner.Role.ShouldBe(ProjectMemberRoles.Admin);

                var dual = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == DualUser);
                dual.ShouldNotBeNull();
                dual.HourlyRate.ShouldBe(RateDualUserA);
                dual.Role.ShouldBe(ProjectMemberRoles.Manager);
            }
        });
    }

    [Fact]
    public async Task Manager_Cannot_Modify_Workspace_Owner_Project_Membership()
    {
        var (wsA, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // DualUser is Manager in Workspace A. OwnerA is Workspace Owner.
        // Manager attempting to modify Owner's project membership role is rejected with 403 RoleNotAllowed
        var req = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={OwnerA}",
            userId: DualUser,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Worker, HourlyRate = RateOwnerA });
        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // Verify OwnerA row in projA1 is completely unchanged
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var owner = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == OwnerA);
                owner.ShouldNotBeNull();
                owner.HourlyRate.ShouldBe(RateOwnerA);
                owner.Role.ShouldBe(ProjectMemberRoles.Admin);
            }
        });
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Spoofing_Target_Header_Cannot_Mutate_Roster()
    {
        var (_, wsB, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ForeignUserA belongs strictly to Workspace A, not Workspace B
        // Attempts to mutate DualUser in projB1 by spoofing X-Workspace-Id: wsB
        var req = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projB1}&userId={DualUser}",
            userId: ForeignUserA,
            workspaceId: wsB,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = 88.00m });
        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Target workspace B rows remain strictly preserved
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var dualB = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == DualUser);
                dualB.ShouldNotBeNull();
                dualB.HourlyRate.ShouldBe(RateDualUserB);
                dualB.Role.ShouldBe(ProjectMemberRoles.Worker);
            }
        });
    }

    [Fact]
    public async Task Workspace_A_Authorized_Header_With_Real_Foreign_Project_And_User_Returns_404_And_Preserves_Foreign_State()
    {
        var (wsA, wsB, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // OwnerA is fully authorized in Workspace A. Passes X-Workspace-Id: wsA but targets real Workspace B project & user
        // Ambient query filter restricts FirstOrDefaultAsync to Workspace A, resulting in member == null -> EntityNotFoundException -> 404
        var req = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projB1}&userId={DualUser}",
            userId: OwnerA,
            workspaceId: wsA,
            jsonBody: new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = 99.00m });
        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // Workspace B sentinels must remain 100% intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var dualB = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projB1 && m.UserId == DualUser);
                dualB.ShouldNotBeNull();
                dualB.HourlyRate.ShouldBe(RateDualUserB);
                dualB.Role.ShouldBe(ProjectMemberRoles.Worker);
            }
        });
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_On_Update_Is_Rejected()
    {
        var (wsA, _, projA1, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var updateDto = new UpdateProjectMemberDto { Role = ProjectMemberRoles.Manager, HourlyRate = 60.00m };

        // 1. Missing workspace header
        // Since UpdateMemberAsync requires [Authorize(TrackerPermissions.Team.Manage)], which is resolved
        // per-workspace, absence of workspace header leaves user unprivileged in aggregate auth -> 403 Forbidden
        var reqMissing = CreateRequest(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}",
            userId: OwnerA,
            workspaceId: null,
            jsonBody: updateDto);
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain("010002");

        // 2. Guid.Empty workspace header
        // Intercepted by production WorkspaceResolutionMiddleware -> 400 BadRequest with WorkspaceRequired
        var reqEmpty = new HttpRequestMessage(
            HttpMethod.Put,
            $"/api/app/team/member?projectId={projA1}&userId={ColleagueA}");
        reqEmpty.Headers.Authorization = new AuthenticationHeaderValue("Bearer", OwnerA.ToString());
        reqEmpty.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        reqEmpty.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());
        var jsonContent = JsonSerializer.Serialize(updateDto);
        reqEmpty.Content = new StringContent(jsonContent, System.Text.Encoding.UTF8, "application/json");

        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);

        // Verify Workspace A sentinels remain unmutated
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var member = await _projectMemberRepository.FirstOrDefaultAsync(
                    m => m.ProjectId == projA1 && m.UserId == ColleagueA);
                member.ShouldNotBeNull();
                member.HourlyRate.ShouldBe(RateColleagueA);
                member.Role.ShouldBe(ProjectMemberRoles.Worker);
            }
        });
    }
}

