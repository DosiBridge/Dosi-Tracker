using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Teams;

public class ProjectMemberDto : CreationAuditedEntityDto<Guid>
{
    public Guid ProjectId { get; set; }
    public Guid UserId { get; set; }
    public string Role { get; set; }
    public decimal HourlyRate { get; set; }
}

public class CreateProjectMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    public Guid ProjectId { get; set; }

    [System.ComponentModel.DataAnnotations.Required]
    public Guid UserId { get; set; }

    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; }

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

    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = "Worker";

    [System.ComponentModel.DataAnnotations.Range(0, 10000)]
    public decimal HourlyRate { get; set; }
}

public class InviteMemberResultDto
{
    public ProjectMemberDto Member { get; set; }

    /// <summary>True when a brand-new Identity user was created for this invite.</summary>
    public bool UserCreated { get; set; }

    /// <summary>One-time initial password for a newly created user. Share it with the
    /// invitee out-of-band; they should change it on first sign-in. Null when the
    /// user already existed.</summary>
    public string? InitialPassword { get; set; }
}

/// <summary>Change an existing project membership. The role is a free-form label ("Worker", "Admin",
/// "Manager", …); Admin/Manager (case-insensitive) also confer the Identity "manager" role.</summary>
public class UpdateProjectMemberDto
{
    [System.ComponentModel.DataAnnotations.Required]
    [System.ComponentModel.DataAnnotations.MaxLength(64)]
    public string Role { get; set; } = "Worker";

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
    public bool IsActive { get; set; }

    /// <summary>Holds the Identity "manager" role (team lead).</summary>
    public bool IsManager { get; set; }

    /// <summary>Holds the Identity "admin" role (workspace owner/administrator).</summary>
    public bool IsOwner { get; set; }

    /// <summary>Projects the user is a member of.</summary>
    public System.Collections.Generic.List<Guid> ProjectIds { get; set; } = new();
}
