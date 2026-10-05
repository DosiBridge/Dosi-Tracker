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

/// <summary>Invite someone by email: finds or creates the Identity user,
/// adds the project membership, and notifies them.</summary>
public class InviteMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    public Guid ProjectId { get; set; }

    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.EmailAddress]
    [System.ComponentModel.DataAnnotations.MaxLength(256)]
    public string Email { get; set; }

    /// <summary>See <see cref="ProjectMemberRoles"/>. Only workspace owners may give Admin/Manager.</summary>
    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = ProjectMemberRoles.Worker;

    /// <summary>Only workspace owners may set a rate other than 0.</summary>
    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

public class InviteMemberResultDto
{
    /// <summary>The new membership; <see cref="ProjectMemberDto.UserName"/> is the account's user name, while the person
    /// signs in with their email address.</summary>
    public ProjectMemberDto Member { get; set; }

    /// <summary>True when a brand-new Identity user was created for this invite.</summary>
    public bool UserCreated { get; set; }

    /// <summary>One-time initial password for a newly created (or re-activated) user. Share it with the
    /// invitee out-of-band; they must change it at their first sign-in. Null when the
    /// user already had an active account.</summary>
    public string? InitialPassword { get; set; }
}

/// <summary>Change an existing project membership (see <see cref="ProjectMemberRoles"/>); Admin/Manager also confer the
/// Identity "manager" role. Only workspace owners may grant Admin/Manager or change the hourly rate, and nobody may
/// change their own membership.</summary>
public class UpdateProjectMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = ProjectMemberRoles.Worker;

    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

/// <summary>A person in the workspace (an Identity user of the current tenant) with their tracker roles.</summary>
public class TeamMemberDto
{
    public Guid UserId { get; set; }
    public string UserName { get; set; } = string.Empty;
    public string? Name { get; set; }
    public string? Surname { get; set; }
    public string? Email { get; set; }

    /// <summary>False once the user was removed from the workspace (deactivated).</summary>
    public bool IsActive { get; set; }

    /// <summary>Holds the Identity "manager" role (team lead).</summary>
    public bool IsManager { get; set; }

    /// <summary>Holds the Identity "admin" role (workspace owner/administrator).</summary>
    public bool IsOwner { get; set; }

    /// <summary>Every membership of the user has the <see cref="ProjectMemberRoles.Client"/> role: a stakeholder who
    /// follows project progress, never tracks time and needs no seat.</summary>
    public bool IsClient { get; set; }

    /// <summary>Projects the user is a member of.</summary>
    public System.Collections.Generic.List<Guid> ProjectIds { get; set; } = new();
}

/// <summary>POST /api/app/team/reset-password result.</summary>
public class ResetPasswordResultDto
{
    /// <summary>The member's new one-time password: share it once, out-of-band; it must be changed at the next sign-in.</summary>
    public string InitialPassword { get; set; } = string.Empty;
}
