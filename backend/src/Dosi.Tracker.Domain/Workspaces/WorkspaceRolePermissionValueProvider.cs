using System;
using System.Linq;
using System.Security.Principal;
using System.Threading.Tasks;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;
using Volo.Abp.Users;

namespace Dosi.Tracker.Workspaces;

/// <summary>Finds a user's active role in a workspace, and checks whether the workspace is archived.</summary>
public class WorkspaceRoleLookup : ITransientDependency
{
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Workspace, Guid>? _workspaceRepository;

    public WorkspaceRoleLookup(
        IRepository<WorkspaceMember, Guid> memberRepository,
        IRepository<Workspace, Guid>? workspaceRepository = null)
    {
        _memberRepository = memberRepository;
        _workspaceRepository = workspaceRepository;
    }

    /// <summary>The user's role in the workspace, or null when they are not an active member of it — or it was deleted
    /// (members of a deleted workspace are members of nothing).</summary>
    [UnitOfWork]
    public virtual async Task<string?> FindRoleAsync(Guid workspaceId, Guid userId)
    {
        var member = await _memberRepository.FirstOrDefaultAsync(m =>
            m.WorkspaceId == workspaceId && m.UserId == userId && m.Status == WorkspaceMemberStatus.Active);
        if (member == null || (_workspaceRepository != null && !await _workspaceRepository.AnyAsync(w => w.Id == workspaceId)))
        {
            return null;
        }

        return member.Role;
    }

    [UnitOfWork]
    public virtual async Task<bool> IsArchivedAsync(Guid workspaceId)
    {
        if (_workspaceRepository == null)
        {
            return false;
        }

        var workspace = await _workspaceRepository.FindAsync(workspaceId);
        return workspace?.IsArchived == true;
    }
}

/// <summary>Grants the permissions of the caller's role in the current workspace (<see cref="WorkspaceRoles"/>).</summary>
public class WorkspaceRolePermissionValueProvider : PermissionValueProvider
{
    public const string ProviderName = "WR";

    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly ICurrentUser _currentUser;
    private readonly WorkspaceRoleLookup _lookup;

    public WorkspaceRolePermissionValueProvider(
        IPermissionStore permissionStore,
        ICurrentWorkspace currentWorkspace,
        ICurrentUser currentUser,
        WorkspaceRoleLookup lookup)
        : base(permissionStore)
    {
        _currentWorkspace = currentWorkspace;
        _currentUser = currentUser;
        _lookup = lookup;
    }

    public override string Name => ProviderName;

    public override async Task<PermissionGrantResult> CheckAsync(PermissionValueCheckContext context)
    {
        var role = await GetRoleAsync(context.Principal?.FindUserId());
        return WorkspaceRoles.PermissionsOf(role).Contains(context.Permission.Name)
            ? PermissionGrantResult.Granted
            : PermissionGrantResult.Undefined;
    }

    public override async Task<MultiplePermissionGrantResult> CheckAsync(PermissionValuesCheckContext context)
    {
        var granted = WorkspaceRoles.PermissionsOf(await GetRoleAsync(context.Principal?.FindUserId()));
        var result = new MultiplePermissionGrantResult();
        foreach (var permission in context.Permissions)
        {
            result.Result[permission.Name] = granted.Contains(permission.Name)
                ? PermissionGrantResult.Granted
                : PermissionGrantResult.Undefined;
        }

        return result;
    }

    private async Task<string?> GetRoleAsync(Guid? userId)
    {
        var workspaceId = _currentWorkspace.Id;
        if (userId == null || workspaceId == null)
        {
            return null;
        }

        if (_currentWorkspace.Role != null && userId == _currentUser.Id)
        {
            return _currentWorkspace.Role;
        }

        return await _lookup.FindRoleAsync(workspaceId.Value, userId.Value);
    }
}
