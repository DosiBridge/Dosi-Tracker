using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Security.Claims;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Presence;

/// <summary>
/// Regressions enforcing that departed or inactive workspace members are excluded from live team presence
/// at the GetTeam query/result boundary, while preserving historical records and legitimate multi-workspace (A+B) presence.
/// </summary>
public class WorkspaceTeamPresenceHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid DepartedUser = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid OwnerB = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid DualUser = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb05");
    private static readonly Guid InactiveUser = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb06");
    private static readonly Guid ForeignNonMember = Guid.Parse("5a701e62-0953-4dd3-910b-dc6cc93ccb07");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly WorkspaceMembershipManager _membershipManager;
    private readonly WorkspaceAppService _workspaceAppService;
    private readonly ICurrentPrincipalAccessor _currentPrincipalAccessor;

    public WorkspaceTeamPresenceHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _presenceRepository = GetRequiredService<IRepository<AgentPresence, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _membershipManager = GetRequiredService<WorkspaceMembershipManager>();
        _workspaceAppService = GetRequiredService<WorkspaceAppService>();
        _currentPrincipalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
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
        return WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
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

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA, Guid ProjectB)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();
        var projA = _guidGenerator.Create();
        var projB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            await CreateUserAsync(OwnerA, "owner_a", "Owner A", "owner_a@test.com");
            await CreateUserAsync(ColleagueA, "colleague_a", "Colleague A", "colleague_a@test.com");
            await CreateUserAsync(DepartedUser, "departed_user", "Departed User", "departed@test.com");
            await CreateUserAsync(OwnerB, "owner_b", "Owner B", "owner_b@test.com");
            await CreateUserAsync(DualUser, "dual_user", "Dual User", "dual@test.com");
            await CreateUserAsync(InactiveUser, "inactive_user", "Inactive User", "inactive@test.com");
            await CreateUserAsync(ForeignNonMember, "foreign_non_member", "Foreign Non Member", "foreign_nm@test.com");

            var workspaceA = new Workspace(wsA, "Workspace Alpha");
            var workspaceB = new Workspace(wsB, "Workspace Beta");
            await _workspaceRepository.InsertAsync(workspaceA, autoSave: true);
            await _workspaceRepository.InsertAsync(workspaceB, autoSave: true);

            var pA = new Project(projA, wsA, "Project Alpha");
            var pB = new Project(projB, wsB, "Project Beta");
            await _projectRepository.InsertAsync(pA, autoSave: true);
            await _projectRepository.InsertAsync(pB, autoSave: true);

            var now = DateTime.UtcNow;

            // Workspace A members: OwnerA (Owner), ColleagueA (Member), DepartedUser (Member), DualUser (Owner)
            var mA1 = new WorkspaceMember(_guidGenerator.Create(), wsA, OwnerA, WorkspaceRoles.Owner, now);
            var mA2 = new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, now);
            var mA3 = new WorkspaceMember(_guidGenerator.Create(), wsA, DepartedUser, WorkspaceRoles.Member, now);
            var mA4 = new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Owner, now);
            await _memberRepository.InsertAsync(mA1, autoSave: true);
            await _memberRepository.InsertAsync(mA2, autoSave: true);
            await _memberRepository.InsertAsync(mA3, autoSave: true);
            await _memberRepository.InsertAsync(mA4, autoSave: true);

            // Workspace B members: OwnerB (Owner), DepartedUser (Member), DualUser (Member)
            var mB1 = new WorkspaceMember(_guidGenerator.Create(), wsB, OwnerB, WorkspaceRoles.Owner, now);
            var mB2 = new WorkspaceMember(_guidGenerator.Create(), wsB, DepartedUser, WorkspaceRoles.Member, now);
            var mB3 = new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, now);
            await _memberRepository.InsertAsync(mB1, autoSave: true);
            await _memberRepository.InsertAsync(mB2, autoSave: true);
            await _memberRepository.InsertAsync(mB3, autoSave: true);
        });

        return (wsA, wsB, projA, projB);
    }

    private async Task CreateUserAsync(Guid id, string userName, string name, string email)
    {
        var existing = await _userManager.FindByIdAsync(id.ToString());
        if (existing == null)
        {
            var user = new IdentityUser(id, userName, email);
            user.Name = name;
            (await _userManager.CreateAsync(user, "Pass@1234")).Succeeded.ShouldBeTrue();
        }
    }

    [Fact]
    public async Task Departed_Member_Removed_From_Workspace_A_Is_Excluded_From_A_Team_Presence_While_Retaining_B_Presence()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var deviceA = _guidGenerator.Create();
        var deviceB = _guidGenerator.Create();
        var colleagueDevice = _guidGenerator.Create();
        var ownerBDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        // 1. Seed recent heartbeats/presence for DepartedUser in both Workspace A and Workspace B
        // along with sentinels for ColleagueA in A and OwnerB in B
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDevice, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                var pDepartedA = new AgentPresence(_guidGenerator.Create(), wsA, DepartedUser, deviceA, "tracking", now)
                {
                    DeviceName = "DEPARTED-PC-A",
                    Platform = "windows",
                    ProjectId = projA
                };
                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
                await _presenceRepository.InsertAsync(pDepartedA, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pOwnerB = new AgentPresence(_guidGenerator.Create(), wsB, OwnerB, ownerBDevice, "tracking", now)
                {
                    DeviceName = "OWNERB-PC",
                    Platform = "windows",
                    ProjectId = projB
                };
                var pDepartedB = new AgentPresence(_guidGenerator.Create(), wsB, DepartedUser, deviceB, "tracking", now)
                {
                    DeviceName = "DEPARTED-PC-B",
                    Platform = "windows",
                    ProjectId = projB
                };
                await _presenceRepository.InsertAsync(pOwnerB, autoSave: true);
                await _presenceRepository.InsertAsync(pDepartedB, autoSave: true);
            }
        });

        // 2. Remove DepartedUser from Workspace A via real production domain membership manager path
        await WithUnitOfWorkAsync(async () =>
        {
            var memberA = await _membershipManager.FindActiveAsync(wsA, DepartedUser);
            memberA.ShouldNotBeNull();
            await _membershipManager.RemoveAsync(memberA);

            // Verify membership status transition to Removed
            var reloadedMemberA = await _membershipManager.FindAsync(wsA, DepartedUser);
            reloadedMemberA.ShouldNotBeNull();
            reloadedMemberA.IsActive.ShouldBeFalse();
            reloadedMemberA.Status.ShouldBe(WorkspaceMemberStatus.Removed);

            // DepartedUser's global IdentityUser remains active
            var globalUser = await _userManager.GetByIdAsync(DepartedUser);
            globalUser.IsActive.ShouldBeTrue();

            // Workspace B membership remains Active
            var memberB = await _membershipManager.FindActiveAsync(wsB, DepartedUser);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
        });

        // 3. Query Workspace A live team presence endpoint as OwnerA
        var reqA = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerA, workspaceId: wsA);
        var respA = await client.SendAsync(reqA);
        respA.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonA = await respA.Content.ReadAsStringAsync();
        using var docA = JsonDocument.Parse(jsonA);
        var itemsA = docA.RootElement.GetProperty("items").EnumerateArray().ToList();

        // DepartedUser MUST NOT appear in Workspace A's team presence list.
        // Only active member ColleagueA should appear!
        var userIdsInA = itemsA.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsInA.ShouldNotContain(DepartedUser);
        userIdsInA.ShouldContain(ColleagueA);

        // 4. Query Workspace B live team presence endpoint as OwnerB
        // DepartedUser remains legitimately active in Workspace B and MUST be present in B
        var reqB = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerB, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonB = await respB.Content.ReadAsStringAsync();
        using var docB = JsonDocument.Parse(jsonB);
        var itemsB = docB.RootElement.GetProperty("items").EnumerateArray().ToList();
        var userIdsInB = itemsB.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsInB.ShouldContain(DepartedUser);
        userIdsInB.ShouldContain(OwnerB);

        // 5. Verify non-destructive preservation: DepartedUser's AgentPresence row in Workspace A was NOT destroyed
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var presenceRowA = await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == DepartedUser);
                presenceRowA.ShouldNotBeNull();
                presenceRowA.DeviceId.ShouldBe(deviceA);
            }
        });
    }

    [Fact]
    public async Task Departed_Member_Via_Self_Service_Leave_Is_Excluded_From_Team_Presence()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var deviceA = _guidGenerator.Create();
        var deviceB = _guidGenerator.Create();
        var colleagueDevice = _guidGenerator.Create();
        var ownerBDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        // 1. Seed presence for DepartedUser in A and B
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDevice, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                var pDepartedA = new AgentPresence(_guidGenerator.Create(), wsA, DepartedUser, deviceA, "tracking", now)
                {
                    DeviceName = "DEPARTED-PC-A",
                    Platform = "windows",
                    ProjectId = projA
                };
                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
                await _presenceRepository.InsertAsync(pDepartedA, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pOwnerB = new AgentPresence(_guidGenerator.Create(), wsB, OwnerB, ownerBDevice, "tracking", now)
                {
                    DeviceName = "OWNERB-PC",
                    Platform = "windows",
                    ProjectId = projB
                };
                var pDepartedB = new AgentPresence(_guidGenerator.Create(), wsB, DepartedUser, deviceB, "tracking", now)
                {
                    DeviceName = "DEPARTED-PC-B",
                    Platform = "windows",
                    ProjectId = projB
                };
                await _presenceRepository.InsertAsync(pOwnerB, autoSave: true);
                await _presenceRepository.InsertAsync(pDepartedB, autoSave: true);
            }
        });

        // 2. DepartedUser leaves Workspace A via self-service LeaveAsync
        using (SetCurrentUser(DepartedUser, "departed@test.com"))
        {
            await _workspaceAppService.LeaveAsync(wsA);
        }

        // Verify membership transition in A is Removed, while B remains Active
        await WithUnitOfWorkAsync(async () =>
        {
            var memberA = await _membershipManager.FindAsync(wsA, DepartedUser);
            memberA.ShouldNotBeNull();
            memberA.IsActive.ShouldBeFalse();
            memberA.Status.ShouldBe(WorkspaceMemberStatus.Removed);

            var memberB = await _membershipManager.FindActiveAsync(wsB, DepartedUser);
            memberB.ShouldNotBeNull();
            memberB.IsActive.ShouldBeTrue();
        });

        // 3. Query Workspace A live team presence: DepartedUser is excluded
        var reqA = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerA, workspaceId: wsA);
        var respA = await client.SendAsync(reqA);
        respA.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonA = await respA.Content.ReadAsStringAsync();
        using var docA = JsonDocument.Parse(jsonA);
        var itemsA = docA.RootElement.GetProperty("items").EnumerateArray().ToList();
        var userIdsInA = itemsA.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsInA.ShouldNotContain(DepartedUser);
        userIdsInA.ShouldContain(ColleagueA);

        // 4. Query Workspace B live team presence: DepartedUser remains present in B
        var reqB = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerB, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonB = await respB.Content.ReadAsStringAsync();
        using var docB = JsonDocument.Parse(jsonB);
        var itemsB = docB.RootElement.GetProperty("items").EnumerateArray().ToList();
        var userIdsInB = itemsB.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsInB.ShouldContain(DepartedUser);
        userIdsInB.ShouldContain(OwnerB);

        // 5. Verify non-destructive preservation in DB
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var presenceRowA = await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == DepartedUser);
                presenceRowA.ShouldNotBeNull();
                presenceRowA.DeviceId.ShouldBe(deviceA);
            }
        });
    }

    [Fact]
    public async Task Inactive_Membership_And_Non_Members_Are_Excluded_While_Current_Active_Members_Are_Retained()
    {
        var (wsA, wsB, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var colleagueDevice = _guidGenerator.Create();
        var inactiveDevice = _guidGenerator.Create();
        var foreignDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        await WithUnitOfWorkAsync(async () =>
        {
            // Set InactiveUser as a member of wsA with Status = Removed
            var mInactive = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveUser, WorkspaceRoles.Member, now.AddDays(-10));
            mInactive.Remove();
            await _memberRepository.InsertAsync(mInactive, autoSave: true);

            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                // Active member presence
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDevice, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                // Inactive member presence
                var pInactive = new AgentPresence(_guidGenerator.Create(), wsA, InactiveUser, inactiveDevice, "tracking", now)
                {
                    DeviceName = "INACTIVE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                // Foreign non-member presence (user with NO membership record at all in wsA)
                var pForeign = new AgentPresence(_guidGenerator.Create(), wsA, ForeignNonMember, foreignDevice, "tracking", now)
                {
                    DeviceName = "FOREIGN-PC",
                    Platform = "windows",
                    ProjectId = projA
                };

                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
                await _presenceRepository.InsertAsync(pInactive, autoSave: true);
                await _presenceRepository.InsertAsync(pForeign, autoSave: true);
            }
        });

        // Query Workspace A team presence as OwnerA
        var req = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerA, workspaceId: wsA);
        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var json = await resp.Content.ReadAsStringAsync();
        using var doc = JsonDocument.Parse(json);
        var items = doc.RootElement.GetProperty("items").EnumerateArray().ToList();
        var userIds = items.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();

        // Active member MUST be retained
        userIds.ShouldContain(ColleagueA);

        // Inactive member MUST be excluded
        userIds.ShouldNotContain(InactiveUser);

        // Foreign non-member MUST be excluded
        userIds.ShouldNotContain(ForeignNonMember);

        // DB presence rows are non-destructively preserved
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                (await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == InactiveUser)).ShouldNotBeNull();
                (await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == ForeignNonMember)).ShouldNotBeNull();
            }
        });
    }

    [Fact]
    public async Task Regular_Member_Without_ViewAll_Only_Sees_Own_Device_In_Live_Presence()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var colleagueDevice = _guidGenerator.Create();
        var ownerDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDevice, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                var pOwner = new AgentPresence(_guidGenerator.Create(), wsA, OwnerA, ownerDevice, "tracking", now)
                {
                    DeviceName = "OWNER-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
                await _presenceRepository.InsertAsync(pOwner, autoSave: true);
            }
        });

        // 1. ColleagueA (Member role, no Activities.ViewAll) queries live team presence
        var reqMember = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: ColleagueA, workspaceId: wsA);
        var respMember = await client.SendAsync(reqMember);
        respMember.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonMember = await respMember.Content.ReadAsStringAsync();
        using var docMember = JsonDocument.Parse(jsonMember);
        var itemsMember = docMember.RootElement.GetProperty("items").EnumerateArray().ToList();

        // Regular member MUST see ONLY their own device
        itemsMember.Count.ShouldBe(1);
        Guid.Parse(itemsMember[0].GetProperty("userId").GetString()!).ShouldBe(ColleagueA);
        Guid.Parse(itemsMember[0].GetProperty("deviceId").GetString()!).ShouldBe(colleagueDevice);

        // 2. OwnerA (Owner role, has Activities.ViewAll) queries live team presence
        var reqOwner = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: OwnerA, workspaceId: wsA);
        var respOwner = await client.SendAsync(reqOwner);
        respOwner.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonOwner = await respOwner.Content.ReadAsStringAsync();
        using var docOwner = JsonDocument.Parse(jsonOwner);
        var itemsOwner = docOwner.RootElement.GetProperty("items").EnumerateArray().ToList();

        // Owner sees BOTH devices
        itemsOwner.Count.ShouldBe(2);
        var userIdsOwnerView = itemsOwner.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsOwnerView.ShouldContain(ColleagueA);
        userIdsOwnerView.ShouldContain(OwnerA);
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Attempting_Presence_Access_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (wsA, wsB, _, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var ownerBDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pOwnerB = new AgentPresence(_guidGenerator.Create(), wsB, OwnerB, ownerBDevice, "tracking", now)
                {
                    DeviceName = "OWNERB-PC",
                    Platform = "windows",
                    ProjectId = projB
                };
                await _presenceRepository.InsertAsync(pOwnerB, autoSave: true);
            }
        });

        // ColleagueA belongs only to Workspace A; attempts to access Workspace B presence
        var req = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: ColleagueA, workspaceId: wsB);
        var response = await client.SendAsync(req);

        response.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await response.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // Verify Workspace B presence is completely untouched
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var presenceB = await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsB && p.UserId == OwnerB);
                presenceB.ShouldNotBeNull();
                presenceB.DeviceId.ShouldBe(ownerBDevice);
            }
        });
    }

    [Fact]
    public async Task Same_User_With_Different_Roles_In_Workspaces_A_And_B_Switches_Legitimately()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var dualDeviceA = _guidGenerator.Create();
        var dualDeviceB = _guidGenerator.Create();
        var colleagueDevice = _guidGenerator.Create();
        var ownerBDevice = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        // DualUser is Owner in wsA and Member in wsB
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDevice, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-PC",
                    Platform = "windows",
                    ProjectId = projA
                };
                var pDualA = new AgentPresence(_guidGenerator.Create(), wsA, DualUser, dualDeviceA, "tracking", now)
                {
                    DeviceName = "DUAL-PC-A",
                    Platform = "windows",
                    ProjectId = projA
                };
                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
                await _presenceRepository.InsertAsync(pDualA, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pOwnerB = new AgentPresence(_guidGenerator.Create(), wsB, OwnerB, ownerBDevice, "tracking", now)
                {
                    DeviceName = "OWNERB-PC",
                    Platform = "windows",
                    ProjectId = projB
                };
                var pDualB = new AgentPresence(_guidGenerator.Create(), wsB, DualUser, dualDeviceB, "tracking", now)
                {
                    DeviceName = "DUAL-PC-B",
                    Platform = "windows",
                    ProjectId = projB
                };
                await _presenceRepository.InsertAsync(pOwnerB, autoSave: true);
                await _presenceRepository.InsertAsync(pDualB, autoSave: true);
            }
        });

        // 1. DualUser in wsA (Owner role: ViewAll granted): sees both ColleagueA and DualUser
        var reqA = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: DualUser, workspaceId: wsA);
        var respA = await client.SendAsync(reqA);
        respA.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonA = await respA.Content.ReadAsStringAsync();
        using var docA = JsonDocument.Parse(jsonA);
        var itemsA = docA.RootElement.GetProperty("items").EnumerateArray().ToList();
        var userIdsInA = itemsA.Select(item => Guid.Parse(item.GetProperty("userId").GetString()!)).ToList();
        userIdsInA.ShouldContain(ColleagueA);
        userIdsInA.ShouldContain(DualUser);

        // 2. DualUser in wsB (Member role: ViewAll NOT granted): sees ONLY own device, OwnerB is excluded!
        var reqB = CreateRequest(HttpMethod.Get, "/api/app/presence/team", userId: DualUser, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);

        var jsonB = await respB.Content.ReadAsStringAsync();
        using var docB = JsonDocument.Parse(jsonB);
        var itemsB = docB.RootElement.GetProperty("items").EnumerateArray().ToList();
        itemsB.Count.ShouldBe(1);
        Guid.Parse(itemsB[0].GetProperty("userId").GetString()!).ShouldBe(DualUser);
        Guid.Parse(itemsB[0].GetProperty("deviceId").GetString()!).ShouldBe(dualDeviceB);
    }
}

