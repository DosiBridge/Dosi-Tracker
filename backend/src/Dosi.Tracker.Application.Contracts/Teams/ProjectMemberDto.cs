using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Teams;

public class ProjectMemberDto : CreationAuditedEntityDto<Guid>
{
    public Guid ProjectId { get; set; }
    public Guid UserId { get; set; }
    public string Role { get; set; }

    /// <summary>Payroll data: 0 for callers without Tracker.Team.Manage.</summary>
    public decimal HourlyRate { get; set; }

    /// <summary>The member's user name ("" when the membership points at a user that no longer exists).</summary>
    public string UserName { get; set; } = string.Empty;

    public string? Name { get; set; }

    public string? Surname { get; set; }

    /// <summary>Only for callers with Tracker.Team.Manage; null for everyone else.</summary>
    public string? Email { get; set; }
}

public class CreateProjectMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    public Guid ProjectId { get; set; }

    /// <summary>An active user of the current workspace (otherwise 404 Tracker:InvalidUser).</summary>
    [System.ComponentModel.DataAnnotations.Required]
    public Guid UserId { get; set; }

    /// <summary>See <see cref="ProjectMemberRoles"/>. Only workspace owners may give Admin/Manager.</summary>
    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; }

    /// <summary>Only workspace owners may set a rate other than 0.</summary>
    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

/// <summary>Invite someone by email (<c>POST /api/app/team/invite-member</c>). Nobody is ever added to the workspace
/// without accepting: whether or not the email has an account, a pending invitation is created and answered with its
/// link (the answer does not reveal whether the account exists). Only an active member of this workspace invited to a
/// project joins that project at once.</summary>
public class InviteMemberDto
{
    /// <summary>Optional: the project the person joins as well.</summary>
    public Guid? ProjectId { get; set; }

    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.EmailAddress]
    [System.ComponentModel.DataAnnotations.MaxLength(256)]
    public string Email { get; set; } = string.Empty;

    /// <summary>The project role (<see cref="ProjectMemberRoles"/>) when <see cref="ProjectId"/> is given. Only
    /// workspace owners may give Admin/Manager.</summary>
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = ProjectMemberRoles.Worker;

    /// <summary>The workspace role: <c>member</c> (default), <c>manager</c> or <c>owner</c> (owners only). An
    /// Admin/Manager project role makes the person a manager at least.</summary>
    [System.ComponentModel.DataAnnotations.MaxLength(16)]
    public string? WorkspaceRole { get; set; }

    /// <summary>Only workspace owners may set a rate other than 0.</summary>
    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

public class InviteMemberResultDto
{
    /// <summary>True only when the person was already an active member of the workspace and was added to the project
    /// at once; false when a pending invitation was created.</summary>
    public bool Added { get; set; }

    /// <summary>The person's workspace role (the role the invitation grants once accepted).</summary>
    public string WorkspaceRole { get; set; } = string.Empty;

    /// <summary>The new project membership, when an existing member was added to the project.</summary>
    public ProjectMemberDto? Member { get; set; }

    /// <summary>The pending invitation the invitee has to accept.</summary>
    public Guid? InvitationId { get; set; }

    /// <summary>Path of the invitation link, <c>/invite/{token}</c> (null when added at once). Share it with the
    /// invitee; the token is shown only here.</summary>
    public string? InvitePath { get; set; }

    public DateTime? ExpiresAt { get; set; }
}

/// <summary>A pending invitation of the current workspace.</summary>
public class WorkspaceInvitationDto
{
    public Guid Id { get; set; }
    public string Email { get; set; } = string.Empty;
    public string Role { get; set; } = string.Empty;
    public Guid? ProjectId { get; set; }
    public string? ProjectRole { get; set; }
    public DateTime ExpiresAt { get; set; }
    public DateTime CreationTime { get; set; }
}

/// <summary>Change an existing project membership (see <see cref="ProjectMemberRoles"/>); Admin/Manager also make the
/// person a workspace manager. Only workspace owners may grant Admin/Manager or change the hourly rate, and nobody may
/// change their own membership.</summary>
public class UpdateProjectMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = ProjectMemberRoles.Worker;

    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

/// <summary>An active member of the current workspace with their roles.</summary>
public class TeamMemberDto
{
    public Guid UserId { get; set; }
    public string UserName { get; set; } = string.Empty;
    public string? Name { get; set; }
    public string? Surname { get; set; }
    public string? Email { get; set; }

    /// <summary>The account is active (members are always active in the workspace).</summary>
    public bool IsActive { get; set; }

    /// <summary><c>owner</c> | <c>manager</c> | <c>member</c>.</summary>
    public string Role { get; set; } = string.Empty;

    public bool IsManager { get; set; }
    public bool IsOwner { get; set; }

    /// <summary>Every project membership of the user has the <see cref="ProjectMemberRoles.Client"/> role.</summary>
    public bool IsClient { get; set; }

    public DateTime JoinedAt { get; set; }

    /// <summary>Projects the user is a member of.</summary>
    public System.Collections.Generic.List<Guid> ProjectIds { get; set; } = new();
}
