using System;
using System.ComponentModel.DataAnnotations;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.SaaS;

/// <summary>Self-service SaaS signup: creates an isolated tenant, seeds its admin
/// user and starts a subscription on the chosen plan.</summary>
public class RegisterWorkspaceDto
{
    [Required]
    [MaxLength(64)]
    public string Name { get; set; }

    [Required]
    [EmailAddress]
    [MaxLength(256)]
    public string AdminEmail { get; set; }

    /// <summary>Must satisfy the password policy. Never written to the audit log.</summary>
    [Required]
    [MaxLength(128)]
    [DisableAuditing]
    public string AdminPassword { get; set; }

    /// <summary>Plan name as listed by <c>GET /api/app/workspace/available-plans</c>; defaults to Free.
    /// Ignored when <see cref="PlanId"/> is given.</summary>
    [MaxLength(64)]
    public string? PlanName { get; set; }

    /// <summary>Id of a plan listed by <c>GET /api/app/workspace/available-plans</c> (optional; preferred over
    /// <see cref="PlanName"/>). Self-service sign-up accepts free plans and plans with a trial.</summary>
    public Guid? PlanId { get; set; }
}

public class WorkspaceRegistrationResultDto
{
    public Guid TenantId { get; set; }
    public string Name { get; set; }
}
