using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>Permissions come from the caller's role in the current workspace: the same person can own one workspace
/// and be a plain member of another.</summary>
public abstract class WorkspacePermissionTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly WorkspaceRolePermissionValueProvider _provider;
    private readonly IPermissionDefinitionManager _definitions;
    private readonly ICurrentWorkspace _currentWorkspace;

    protected WorkspacePermissionTests()
    {
        _provider = GetRequiredService<WorkspaceRolePermissionValueProvider>();
        _definitions = GetRequiredService<IPermissionDefinitionManager>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    private static ClaimsPrincipal Principal(Guid userId)
    {
        return new ClaimsPrincipal(new ClaimsIdentity(new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) }, "Test"));
    }

    private async Task<bool> IsGrantedAsync(Guid userId, string permission)
    {
        var definition = await _definitions.GetAsync(permission);
        return await WithUnitOfWorkAsync(async () =>
            await _provider.CheckAsync(new PermissionValueCheckContext(definition, Principal(userId))) == PermissionGrantResult.Granted);
    }

    private async Task<Dictionary<string, PermissionGrantResult>> CheckAllAsync(Guid userId, IEnumerable<string> permissions)
    {
        var definitions = new List<PermissionDefinition>();
        foreach (var name in permissions)
        {
            definitions.Add(await _definitions.GetAsync(name));
        }

        var result = await WithUnitOfWorkAsync(() =>
            _provider.CheckAsync(new PermissionValuesCheckContext(definitions, Principal(userId))));
        return result.Result;
    }

    [Fact]
    public void The_Provider_Should_Be_Registered()
    {
        GetRequiredService<IOptions<AbpPermissionOptions>>().Value.ValueProviders
            .ShouldContain(typeof(WorkspaceRolePermissionValueProvider));
    }

    [Fact]
    public async Task The_Same_User_Should_Have_Owner_Rights_In_One_Workspace_And_Member_Rights_In_Another()
    {
        var userId = Guid.NewGuid();
        var a = await ServiceProvider.CreateWorkspaceAsync("perm-a");
        var b = await ServiceProvider.CreateWorkspaceAsync("perm-b");
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Owner, a);
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Member, b);

        using (_currentWorkspace.Change(a))
        {
            (await IsGrantedAsync(userId, TrackerPermissions.Billing.Default)).ShouldBeTrue();
            (await IsGrantedAsync(userId, TrackerPermissions.Team.Manage)).ShouldBeTrue();
            (await IsGrantedAsync(userId, TrackerPermissions.Projects.Delete)).ShouldBeTrue();
            (await IsGrantedAsync(userId, TrackerPermissions.Platform.Default)).ShouldBeFalse();
        }

        using (_currentWorkspace.Change(b))
        {
            (await IsGrantedAsync(userId, TrackerPermissions.Billing.Default)).ShouldBeFalse();
            (await IsGrantedAsync(userId, TrackerPermissions.Team.Manage)).ShouldBeFalse();
            (await IsGrantedAsync(userId, TrackerPermissions.Activities.ViewAll)).ShouldBeFalse();
        }

        using (_currentWorkspace.Change(null))
        {
            (await IsGrantedAsync(userId, TrackerPermissions.Team.Manage)).ShouldBeFalse();
        }
    }

    [Fact]
    public async Task A_Manager_Should_Get_The_Team_Lead_Permissions_Only()
    {
        var userId = Guid.NewGuid();
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Manager);

        var all = WorkspaceRoles.OwnerPermissions;
        var result = await CheckAllAsync(userId, all);

        result.Where(r => r.Value == PermissionGrantResult.Granted).Select(r => r.Key)
            .ShouldBe(WorkspaceRoles.ManagerPermissions, ignoreOrder: true);
        result[TrackerPermissions.Billing.Default].ShouldBe(PermissionGrantResult.Undefined);
        result[TrackerPermissions.Projects.Delete].ShouldBe(PermissionGrantResult.Undefined);
    }

    [Fact]
    public async Task A_Removed_Member_Should_Have_No_Rights()
    {
        var userId = Guid.NewGuid();
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Owner);
        await ServiceProvider.AddWorkspaceMemberAsync(Guid.NewGuid(), WorkspaceRoles.Owner); // the workspace keeps an owner
        await WithUnitOfWorkAsync(async () =>
        {
            var memberships = GetRequiredService<WorkspaceMembershipManager>();
            await memberships.RemoveAsync((await memberships.FindAsync(TrackerTestConsts.WorkspaceId, userId))!);
        });

        (await IsGrantedAsync(userId, TrackerPermissions.Billing.Default)).ShouldBeFalse();
    }

    [Fact]
    public async Task The_Role_Resolved_For_The_Request_Should_Be_Used_For_The_Caller()
    {
        // The X-Workspace-Id middleware resolves the caller's role once; application-configuration reads it from there.
        var callerId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d"); // FakeCurrentPrincipalAccessor
        using (_currentWorkspace.Change(TrackerTestConsts.WorkspaceId, WorkspaceRoles.Owner))
        {
            (await IsGrantedAsync(callerId, TrackerPermissions.Billing.Default)).ShouldBeTrue();
            (await IsGrantedAsync(Guid.NewGuid(), TrackerPermissions.Billing.Default)).ShouldBeFalse(); // someone else
        }
    }
}
