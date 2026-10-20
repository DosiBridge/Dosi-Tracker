using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// REQ-ISO-01: EF Core Entity Query Scoping & Repository Data Isolation.
/// Verifies that EF Core's IWorkspaceScoped global query filters and repository queries
/// restrict entity retrieval, modification, deletion, and cross-entity references strictly
/// to the active workspace ambiently set on ICurrentWorkspace.
/// NOTE: This suite tests data/query isolation under ambient context; HTTP header-to-role
/// admission and production permission checks are exercised in WorkspaceResolutionIntegrationTests.
/// </summary>
public abstract class CrossWorkspaceIsolationTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid UserA = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d"); // CurrentUserId in FakeCurrentPrincipalAccessor
    private static readonly Guid UserB = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000003");
    private static readonly Guid DualUser = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000004");
    private static readonly DateTime Day = new(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);

    private readonly IProjectAppService _projectAppService;
    private readonly IActivityAppService _activityAppService;
    private readonly ITeamAppService _teamAppService;
    private readonly IPresenceAppService _presenceAppService;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly WorkspaceRoleLookup _workspaceRoleLookup;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentPrincipalAccessor _principalAccessor;
    private readonly IdentityUserManager _userManager;

    protected CrossWorkspaceIsolationTests()
    {
        _projectAppService = GetRequiredService<IProjectAppService>();
        _activityAppService = GetRequiredService<IActivityAppService>();
        _teamAppService = GetRequiredService<ITeamAppService>();
        _presenceAppService = GetRequiredService<IPresenceAppService>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _presenceRepository = GetRequiredService<IRepository<AgentPresence, Guid>>();
        _workspaceRoleLookup = GetRequiredService<WorkspaceRoleLookup>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
        _userManager = GetRequiredService<IdentityUserManager>();
    }

    private IDisposable ActAs(Guid userId)
    {
        var claims = new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) };
        return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "Test")));
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA, Guid ProjectB, Guid ActivityB, Guid ScreenshotB)> SeedTwoWorkspacesAsync()
    {
        if (await _userManager.FindByIdAsync(UserA.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserA, "userA", "userA@acme.test"));
        }
        if (await _userManager.FindByIdAsync(UserB.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserB, "userB", "userB@acme.test"));
        }
        if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(DualUser, "dualUser", "dualUser@acme.test"));
        }

        var wsA = await ServiceProvider.CreateWorkspaceAsync("Isolation-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("Isolation-WS-B");

        await ServiceProvider.AddWorkspaceMemberAsync(UserA, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);

        var projA = _guidGenerator.Create();
        var projB = _guidGenerator.Create();
        var actB = _guidGenerator.Create();
        var scB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            // Workspace A data
            await _projectRepository.InsertAsync(new Project(projA, wsA, "Project A"));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA, UserA));

            // Workspace B data
            await _projectRepository.InsertAsync(new Project(projB, wsB, "Project B"));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB, UserB));

            var activityB = new Activity(actB, wsB, UserB, projB, Guid.NewGuid(), Day, Day.AddMinutes(10));
            await _activityRepository.InsertAsync(activityB);

            var screenshotB = new Screenshot(scB, wsB, actB, "storage-url-b", Day, 1024, "image/jpeg", Screenshot.ScreenKind);
            await _screenshotRepository.InsertAsync(screenshotB);
        });

        return (wsA, wsB, projA, projB, actB, scB);
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_Get_Project_Owned_By_Workspace_B()
    {
        var (wsA, _, _, projB, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            await Should.ThrowAsync<EntityNotFoundException>(() => _projectAppService.GetAsync(projB));
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_Update_Project_Owned_By_Workspace_B()
    {
        var (wsA, _, _, projB, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            await Should.ThrowAsync<EntityNotFoundException>(() => _projectAppService.UpdateAsync(projB, new CreateUpdateProjectDto
            {
                Title = "Tampered Title",
                Color = "#006bff",
                IntervalMinutes = 10
            }));
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_Delete_Project_Owned_By_Workspace_B()
    {
        var (wsA, wsB, _, projB, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            // Deleting a foreign project ID in Workspace A is a no-op / not found
            await _projectAppService.DeleteAsync(projB);
        }

        // Project B must still exist intact in Workspace B
        using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
        {
            var projectB = await _projectAppService.GetAsync(projB);
            projectB.ShouldNotBeNull();
            projectB.Title.ShouldBe("Project B");
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Project_List_Does_Not_Leak_Workspace_B_Projects()
    {
        var (wsA, _, projA, projB, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            var list = await _projectAppService.GetListAsync(new PagedAndSortedResultRequestDto());
            list.Items.Any(p => p.Id == projA).ShouldBeTrue();
            list.Items.Any(p => p.Id == projB).ShouldBeFalse("Workspace B project must never leak into Workspace A project list");
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_Upload_Activity_To_Project_In_Workspace_B()
    {
        var (wsA, _, _, projB, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            var input = new CreateActivityDto
            {
                ProjectId = projB,
                ClientActivityId = Guid.NewGuid(),
                StartedAt = Day,
                EndedAt = Day.AddMinutes(10),
                Productivity = 80,
                MouseClicks = 50,
                KeyboardHits = 100
            };

            var ex = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.NotProjectMember);
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_Access_Screenshot_Content_Owned_By_Workspace_B()
    {
        var (wsA, _, _, _, _, scB) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            await Should.ThrowAsync<EntityNotFoundException>(() => _activityAppService.GetScreenshotContentAsync(scB));
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Cannot_See_Or_Remove_Workspace_B_Team_Members()
    {
        var (wsA, _, _, _, _, _) = await SeedTwoWorkspacesAsync();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            var members = await _teamAppService.GetMembersAsync();
            members.Items.Any(m => m.UserId == UserA).ShouldBeTrue();
            members.Items.Any(m => m.UserId == UserB).ShouldBeFalse("Workspace B member must not be visible in Workspace A team roster");

            var ex = await Should.ThrowAsync<BusinessException>(() => _teamAppService.RemoveFromWorkspaceAsync(UserB));
            ex.Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
        }
    }

    [Fact]
    public async Task Caller_In_Workspace_A_Heartbeat_Does_Not_Attach_To_Workspace_B_Project()
    {
        var (wsA, _, _, projB, _, _) = await SeedTwoWorkspacesAsync();
        var deviceId = _guidGenerator.Create();

        using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
        {
            var heartbeat = new HeartbeatDto
            {
                DeviceId = deviceId,
                Status = "tracking",
                ProjectId = projB
            };

            var result = await _presenceAppService.HeartbeatAsync(heartbeat);
            result.ShouldNotBeNull();

            // Presence was recorded in Workspace A, but ProjectId must be null because projB belongs to Workspace B
            var presence = await WithUnitOfWorkAsync(() =>
                _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == UserA && p.DeviceId == deviceId));
            presence.ShouldNotBeNull();
            presence.ProjectId.ShouldBeNull("Heartbeat must not link to cross-workspace project");
        }
    }

    [Fact]
    public async Task Dual_Member_Data_Context_Switching_Excludes_Foreign_Workspace_Data()
    {
        var (wsA, wsB, projA, projB, _, _) = await SeedTwoWorkspacesAsync();

        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsB);

        using (ActAs(DualUser))
        {
            // Under Workspace A context: Project A visible, Project B invisible
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var listA = await _projectAppService.GetListAsync(new PagedAndSortedResultRequestDto());
                listA.Items.Any(p => p.Id == projA).ShouldBeTrue();
                listA.Items.Any(p => p.Id == projB).ShouldBeFalse("Workspace B project must never leak into Workspace A context");
            }

            // Switch context to Workspace B: Project B visible, Project A invisible
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Member))
            {
                var listB = await _projectAppService.GetListAsync(new PagedAndSortedResultRequestDto());
                listB.Items.Any(p => p.Id == projA).ShouldBeFalse("Workspace A project must never leak into Workspace B context");
                listB.Items.Any(p => p.Id == projB).ShouldBeTrue();
            }
        }
    }

    [Fact]
    public async Task WorkspaceRoleLookup_Trace_Direct_Repository_Check()
    {
        var (wsA, wsB, _, _, _, _) = await SeedTwoWorkspacesAsync();

        // 1. User A is active Owner in Workspace A
        var roleA = await _workspaceRoleLookup.FindRoleAsync(wsA, UserA);
        roleA.ShouldBe(WorkspaceRoles.Owner);

        // 2. User A is NOT a member of Workspace B
        var roleBForA = await _workspaceRoleLookup.FindRoleAsync(wsB, UserA);
        roleBForA.ShouldBeNull("User A is not a member of Workspace B");

        // 3. User B is NOT a member of Workspace A
        var roleAForB = await _workspaceRoleLookup.FindRoleAsync(wsA, UserB);
        roleAForB.ShouldBeNull("User B is not a member of Workspace A");

        // 4. Guid.Empty workspace query returns null
        var roleEmpty = await _workspaceRoleLookup.FindRoleAsync(Guid.Empty, UserA);
        roleEmpty.ShouldBeNull("Guid.Empty must never match any workspace member");
    }
}
