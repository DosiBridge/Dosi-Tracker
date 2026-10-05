using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Platform;

/// <summary>
/// Platform-owner (host) console: cross-tenant metrics, revenue, and plan management. Every method
/// is host-only — a tenant-scoped caller is rejected. Cross-tenant reads run with the multi-tenancy
/// data filter disabled so the host sees every tenant's rows.
/// </summary>
public interface IPlatformAppService : IApplicationService
{
    Task<PlatformOverviewDto> GetOverviewAsync();

    Task<PagedResultDto<PlatformInvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input);

    Task<PagedResultDto<PlatformUserDto>> GetUsersAsync(PagedAndSortedResultRequestDto input);

    Task<ListResultDto<PlanDto>> GetPlansAsync();

    Task<PlanDto> CreatePlanAsync(CreateUpdatePlanDto input);

    Task<PlanDto> UpdatePlanAsync(Guid id, CreateUpdatePlanDto input);

    Task DeletePlanAsync(Guid id);

    /// <summary>GET /api/app/platform/tenant-subscriptions — every workspace with its plan, subscription status and
    /// occupied seats, by workspace name.</summary>
    Task<ListResultDto<TenantSubscriptionDto>> GetTenantSubscriptionsAsync();

    /// <summary>POST /api/app/platform/change-tenant-plan?tenantId=&amp;planId= — moves a workspace to another plan. A
    /// free plan makes the subscription active; on a paid plan its status is kept. Refused
    /// (<c>Tracker:SeatLimitReached</c>) when the workspace occupies more seats than the plan allows.</summary>
    Task<SubscriptionDto> ChangeTenantPlanAsync(Guid tenantId, Guid planId);

    /// <summary>POST /api/app/platform/activate-subscription?tenantId= — marks a workspace's subscription active (ends
    /// a trial, lifts past_due or a suspension).</summary>
    Task<SubscriptionDto> ActivateSubscriptionAsync(Guid tenantId);

    /// <summary>POST /api/app/platform/suspend-subscription?tenantId= — suspends a workspace: it can no longer record
    /// time until activated again.</summary>
    Task<SubscriptionDto> SuspendSubscriptionAsync(Guid tenantId);

    /// <summary>POST /api/app/platform/mark-invoice-paid?invoiceId= — records an invoice as paid. Paying a workspace's
    /// last overdue invoice makes it active again.</summary>
    Task<PlatformInvoiceDto> MarkInvoicePaidAsync(Guid invoiceId);
}
