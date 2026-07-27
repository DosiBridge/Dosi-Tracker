using System;
using System.ComponentModel.DataAnnotations;

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

    [Required]
    [MaxLength(128)]
    public string AdminPassword { get; set; }

    /// <summary>Plan name ("Free", "Starter", "Business"); defaults to Free.</summary>
    [MaxLength(64)]
    public string? PlanName { get; set; }
}

public class WorkspaceRegistrationResultDto
{
    public Guid TenantId { get; set; }
    public string Name { get; set; }
}
