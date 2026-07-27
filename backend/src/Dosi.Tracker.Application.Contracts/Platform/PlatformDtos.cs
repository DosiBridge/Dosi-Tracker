using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Platform;

/// <summary>Headline metrics for the platform-owner dashboard, aggregated across every tenant.</summary>
public class PlatformOverviewDto
{
    public int TenantCount { get; set; }
    public int ActiveSubscriptions { get; set; }
    public int TrialingSubscriptions { get; set; }

    /// <summary>Monthly recurring revenue: sum over active/trialing subscriptions of the plan's
    /// per-user price times that tenant's occupied seats (min one seat).</summary>
    public decimal Mrr { get; set; }

    /// <summary>Total occupied (billable) seats across active/trialing subscriptions.</summary>
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
    public string TenantName { get; set; } = string.Empty;
    public decimal Amount { get; set; }
    public string Status { get; set; } = string.Empty;
    public DateTime DueDate { get; set; }
}

/// <summary>A user account resolved to its owning tenant, for the cross-tenant users view.</summary>
public class PlatformUserDto
{
    public Guid Id { get; set; }
    public string UserName { get; set; } = string.Empty;
    public string? Email { get; set; }
    public string? Name { get; set; }
    public Guid? TenantId { get; set; }
    public string TenantName { get; set; } = string.Empty;
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
