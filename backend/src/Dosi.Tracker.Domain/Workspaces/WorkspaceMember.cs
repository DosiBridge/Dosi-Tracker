using System;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Workspaces;

/// <summary>A person's membership of a workspace: the source of truth for who is in it and with what role.</summary>
public class WorkspaceMember : CreationAuditedEntity<Guid>
{
    public Guid WorkspaceId { get; private set; }
    public Guid UserId { get; private set; }

    /// <summary>One of <see cref="WorkspaceRoles"/>.</summary>
    public string Role { get; private set; } = null!;

    /// <summary>One of <see cref="WorkspaceMemberStatus"/>.</summary>
    public string Status { get; private set; } = null!;

    public DateTime JoinedAt { get; private set; }

    protected WorkspaceMember()
    {
    }

    public WorkspaceMember(Guid id, Guid workspaceId, Guid userId, string role, DateTime joinedAt)
        : base(id)
    {
        WorkspaceId = workspaceId;
        UserId = userId;
        SetRole(role);
        Status = WorkspaceMemberStatus.Active;
        JoinedAt = joinedAt;
    }

    public bool IsActive => Status == WorkspaceMemberStatus.Active;

    public bool IsOwner => IsActive && Role == WorkspaceRoles.Owner;

    public void SetRole(string role)
    {
        if (!WorkspaceRoles.IsValid(role))
        {
            throw new ArgumentException($"Unknown workspace role '{role}'.", nameof(role));
        }

        Role = role;
    }

    public void Remove()
    {
        Status = WorkspaceMemberStatus.Removed;
    }

    public void Rejoin(string role, DateTime joinedAt)
    {
        SetRole(role);
        Status = WorkspaceMemberStatus.Active;
        JoinedAt = joinedAt;
    }
}
