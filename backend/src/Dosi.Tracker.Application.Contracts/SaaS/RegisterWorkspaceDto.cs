using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.SaaS;

/// <summary>Self-service sign-up (<c>POST /api/app/workspace/register</c>): creates the account and either a new
/// workspace it owns, or — with <see cref="InviteToken"/> — joins the workspace that invited this email.</summary>
public class RegisterWorkspaceDto : IValidatableObject
{
    [MaxLength(64)]
    public string? FirstName { get; set; }

    [MaxLength(64)]
    public string? Surname { get; set; }

    [Required]
    [EmailAddress]
    [MaxLength(256)]
    public string Email { get; set; } = string.Empty;

    /// <summary>Must satisfy the password policy. Never written to the audit log.</summary>
    [Required]
    [MaxLength(128)]
    [DisableAuditing]
    public string Password { get; set; } = string.Empty;

    /// <summary>Required unless <see cref="InviteToken"/> is given.</summary>
    [MaxLength(WorkspaceConsts.MaxNameLength)]
    public string? WorkspaceName { get; set; }

    /// <summary>Id of a plan listed by <c>GET /api/app/workspace/available-plans</c> (optional; preferred over
    /// <see cref="PlanName"/>). Self-service sign-up accepts free plans and plans with a trial.</summary>
    public Guid? PlanId { get; set; }

    /// <summary>Plan name; defaults to Free. Ignored when <see cref="PlanId"/> is given.</summary>
    [MaxLength(64)]
    public string? PlanName { get; set; }

    /// <summary>The token of an invitation sent to <see cref="Email"/>: the account joins that workspace instead of
    /// creating one.</summary>
    [MaxLength(128)]
    public string? InviteToken { get; set; }

    public IEnumerable<ValidationResult> Validate(ValidationContext validationContext)
    {
        if (string.IsNullOrWhiteSpace(InviteToken) && string.IsNullOrWhiteSpace(WorkspaceName))
        {
            yield return new ValidationResult("The workspace name is required.", new[] { nameof(WorkspaceName) });
        }
    }
}

public class WorkspaceRegistrationResultDto
{
    public Guid WorkspaceId { get; set; }
    public string WorkspaceName { get; set; } = string.Empty;

    /// <summary>The new account's user name (its email); sign in with the email.</summary>
    public string UserName { get; set; } = string.Empty;

    /// <summary>Whether an email asking to confirm the address was sent (false when this server sends no email).</summary>
    public bool EmailConfirmationSent { get; set; }
}

/// <summary>A workspace the signed-in user is an active member of (<c>GET /api/app/workspaces/mine</c>).</summary>
public class MyWorkspaceDto
{
    public Guid Id { get; set; }
    public string Name { get; set; } = string.Empty;

    /// <summary><c>owner</c> | <c>manager</c> | <c>member</c>.</summary>
    public string Role { get; set; } = string.Empty;

    public string? PlanName { get; set; }
    public bool IsOwner { get; set; }
    public bool IsArchived { get; set; }
    public DateTime? ArchivedTime { get; set; }
}

public class WorkspaceDto
{
    public Guid Id { get; set; }
    public string Name { get; set; } = string.Empty;
    public bool IsArchived { get; set; }
    public DateTime? ArchivedTime { get; set; }
    public Guid? ArchivedBy { get; set; }
}

public class CreateWorkspaceDto
{
    [Required]
    [MaxLength(WorkspaceConsts.MaxNameLength)]
    public string Name { get; set; } = string.Empty;

    public Guid? PlanId { get; set; }
}

/// <summary><c>POST /api/app/workspaces/{id}/delete</c>: the workspace's name, typed by the owner to confirm.</summary>
public class DeleteWorkspaceDto
{
    [Required]
    [MaxLength(WorkspaceConsts.MaxNameLength)]
    public string ConfirmationName { get; set; } = string.Empty;
}

/// <summary>A deleted workspace: gone at once for everyone; its data is purged after <see cref="PurgeAfter"/>.</summary>
public class DeletedWorkspaceDto
{
    public Guid Id { get; set; }
    public DateTime DeletionTime { get; set; }

    /// <summary>From when the erasure sweep purges the data (the end of the grace period).</summary>
    public DateTime PurgeAfter { get; set; }
}

/// <summary><c>GET /api/app/workspace/billing-mode</c> (anonymous): whether this deployment bills workspaces. When it
/// does not, every workspace is active, there are no trials, seat limits or invoices, and the web app shows no plans or
/// billing.</summary>
public class BillingModeDto
{
    public bool Enabled { get; set; }
}

public class CreatedWorkspaceDto
{
    public Guid Id { get; set; }
    public string Name { get; set; } = string.Empty;
}

/// <summary><c>GET /api/app/invitations/{token}</c> (anonymous).</summary>
public class InvitationInfoDto
{
    public string WorkspaceName { get; set; } = string.Empty;
    public string Email { get; set; } = string.Empty;

    /// <summary>The workspace role the invitee gets.</summary>
    public string Role { get; set; } = string.Empty;

    /// <summary>True once it expired or was used.</summary>
    public bool Expired { get; set; }
}

public class AcceptInvitationResultDto
{
    public Guid WorkspaceId { get; set; }
    public string WorkspaceName { get; set; } = string.Empty;
    public string Role { get; set; } = string.Empty;
}

/// <summary><c>GET /api/app/invitations/mine</c>: the signed-in user's pending invitations. They are matched by the
/// account's email, so they are listed only once that email is verified (<see cref="EmailConfirmed"/>); until then the
/// list is empty and an invitation can be opened only with its link.</summary>
public class MyInvitationsDto
{
    /// <summary>Whether the account's email is verified. False: <see cref="Items"/> is empty.</summary>
    public bool EmailConfirmed { get; set; }

    /// <summary>Usable invitations to workspaces that are not archived, newest first.</summary>
    public List<MyInvitationDto> Items { get; set; } = new();
}

/// <summary>A pending invitation addressed to the signed-in user's email; accept or decline it by its
/// <see cref="Id"/> (<c>POST /api/app/invitations/mine/{id}/accept|decline</c>).</summary>
public class MyInvitationDto
{
    public Guid Id { get; set; }
    public Guid WorkspaceId { get; set; }
    public string WorkspaceName { get; set; } = string.Empty;

    /// <summary>The workspace role the invitation grants.</summary>
    public string Role { get; set; } = string.Empty;

    public DateTime ExpiresAt { get; set; }
    public DateTime CreationTime { get; set; }
}
