using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Platform;

/// <summary>Headline metrics for the platform-owner dashboard, aggregated across every existing tenant
/// (subscriptions of deleted workspaces are left out).</summary>
public class PlatformOverviewDto
{
    public int TenantCount { get; set; }
    public int ActiveSubscriptions { get; set; }
    public int TrialingSubscriptions { get; set; }

    /// <summary>Monthly recurring revenue: sum over ACTIVE subscriptions of the plan's per-user price
    /// times that tenant's occupied seats (min one seat). Trialing subscriptions pay nothing and are excluded.</summary>
    public decimal Mrr { get; set; }

    /// <summary>Total occupied (billable) seats across active subscriptions on paid plans; seats on free plans
    /// pay nothing and are not counted.</summary>
    public int PaidSeats { get; set; }

    /// <summary>Sum of every invoice ever raised, across all tenants.</summary>
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

/// <summary>An invoice with its owning tenant resolved, for the platform billing view.</summary>
public class PlatformInvoiceDto
{
    public Guid Id { get; set; }
    public Guid? TenantId { get; set; }

    /// <summary>The workspace's name; "Host" without a tenant, "Deleted workspace" when the tenant no longer exists.</summary>
    public string TenantName { get; set; } = string.Empty;

    public decimal Amount { get; set; }
    public string Status { get; set; } = string.Empty;
    public DateTime DueDate { get; set; }

    /// <summary>UTC start of the billed month (unique per tenant); null on invoices from before billing periods.</summary>
    public DateTime? BillingPeriodStart { get; set; }
}

/// <summary>A user account resolved to its owning tenant, for the cross-tenant users view.</summary>
public class PlatformUserDto
{
    public Guid Id { get; set; }
    public string UserName { get; set; } = string.Empty;
    public string? Email { get; set; }
    public string? Name { get; set; }
    public Guid? TenantId { get; set; }

    /// <summary>The workspace's name; "Host" without a tenant, "Deleted workspace" when the tenant no longer exists.</summary>
    public string TenantName { get; set; } = string.Empty;
}

/// <summary>A workspace with its subscription, for the host's tenant list and plan management.</summary>
public class TenantSubscriptionDto
{
    public Guid TenantId { get; set; }
    public string TenantName { get; set; } = string.Empty;

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
