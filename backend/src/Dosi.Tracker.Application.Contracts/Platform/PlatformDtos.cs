using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Platform;

/// <summary>Headline metrics for the platform-owner dashboard, aggregated across every workspace.</summary>
public class PlatformOverviewDto
{
    public int WorkspaceCount { get; set; }
    public int ActiveSubscriptions { get; set; }
    public int TrialingSubscriptions { get; set; }

    /// <summary>Monthly recurring revenue: sum over ACTIVE subscriptions of the plan's per-user price
    /// times that workspace's occupied seats (min one seat). Trialing subscriptions pay nothing and are excluded.</summary>
    public decimal Mrr { get; set; }

    /// <summary>Total occupied (billable) seats across active subscriptions on paid plans; seats on free plans
    /// pay nothing and are not counted.</summary>
    public int PaidSeats { get; set; }

    /// <summary>Sum of every invoice ever raised, across all workspaces.</summary>
    public decimal TotalInvoiced { get; set; }

    public int PendingInvoices { get; set; }

    /// <summary>Per-plan subscriber counts.</summary>
    public List<PlanSubscriberDto> Plans { get; set; } = new();
}

public class PlanSubscriberDto
{
    public Guid PlanId { get; set; }
    public string Name { get; set; } = string.Empty;
    public decimal PricePerUser { get; set; }
    public int SubscriberCount { get; set; }
}

/// <summary>An invoice with its workspace resolved, for the platform billing view.</summary>
public class PlatformInvoiceDto
{
    public Guid Id { get; set; }
    public Guid WorkspaceId { get; set; }

    /// <summary>The workspace's name; "Deleted workspace" when it no longer exists.</summary>
    public string WorkspaceName { get; set; } = string.Empty;

    public decimal Amount { get; set; }
    public string Status { get; set; } = string.Empty;
    public DateTime DueDate { get; set; }

    /// <summary>UTC start of the billed month (unique per workspace); null on invoices from before billing periods.</summary>
    public DateTime? BillingPeriodStart { get; set; }
}

/// <summary>A user account (global) with the workspaces it is an active member of.</summary>
public class PlatformUserDto
{
    public Guid Id { get; set; }
    public string UserName { get; set; } = string.Empty;
    public string? Email { get; set; }
    public string? Name { get; set; }
    public bool IsActive { get; set; }

    /// <summary>Names of the workspaces the user is an active member of, by name.</summary>
    public List<string> WorkspaceNames { get; set; } = new();
}

/// <summary>A workspace with its subscription, for the platform's workspace list and plan management.</summary>
public class WorkspaceSubscriptionDto
{
    public Guid WorkspaceId { get; set; }
    public string WorkspaceName { get; set; } = string.Empty;

    /// <summary>Null (as are the plan and status fields) when the workspace has no subscription.</summary>
    public Guid? SubscriptionId { get; set; }

    public Guid? PlanId { get; set; }
    public string? PlanName { get; set; }
    public string? Status { get; set; }
    public DateTime? TrialEndsAt { get; set; }

    /// <summary>Occupied seats (the same count that is invoiced and limited by the plan).</summary>
    public int SeatsUsed { get; set; }

    /// <summary>The plan's seat limit; null without a known plan.</summary>
    public int? MaxSeats { get; set; }
}

public class CreateUpdatePlanDto
{
    [Required]
    [StringLength(64)]
    public string Name { get; set; } = string.Empty;

    [Range(0, 100000)]
    public decimal PricePerUser { get; set; }

    [Range(1, 100000)]
    public int MaxSeats { get; set; }

    [Range(0, 365)]
    public int TrialDays { get; set; }
}
