using System;
using System.Threading;
using Volo.Abp;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.Workspaces;

/// <summary>The workspace the current request (or code block) works in, and the caller's role there.</summary>
public interface ICurrentWorkspace
{
    Guid? Id { get; }

    /// <summary>The caller's role in the workspace when it was resolved from the request; null otherwise.</summary>
    string? Role { get; }

    IDisposable Change(Guid? id, string? role = null);
}

public class CurrentWorkspace : ICurrentWorkspace, ISingletonDependency
{
    private static readonly AsyncLocal<WorkspaceScope?> Scope = new();

    public virtual Guid? Id => Scope.Value != null ? Scope.Value.Id : DefaultId;

    public virtual string? Role => Scope.Value?.Role;

    /// <summary>The workspace outside any <see cref="Change"/> scope (none).</summary>
    protected virtual Guid? DefaultId => null;

    public IDisposable Change(Guid? id, string? role = null)
    {
        var previous = Scope.Value;
        Scope.Value = new WorkspaceScope(id, role);
        return new DisposeAction(() => Scope.Value = previous);
    }

    private sealed record WorkspaceScope(Guid? Id, string? Role);
}

public static class CurrentWorkspaceExtensions
{
    /// <summary>The current workspace's id; <c>Tracker:WorkspaceRequired</c> without one.</summary>
    public static Guid GetId(this ICurrentWorkspace currentWorkspace)
    {
        return currentWorkspace.Id ?? throw new BusinessException(TrackerDomainErrorCodes.WorkspaceRequired);
    }
}
