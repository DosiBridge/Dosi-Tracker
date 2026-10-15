using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Platform;

/// <summary>
/// Platform console (global <c>platform-admin</c> role): metrics, revenue and plan management across every workspace.
/// Reads run with the workspace data filter disabled.
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

    /// <summary>GET /api/app/platform/workspace-subscriptions — every workspace with its plan, subscription status and
    /// occupied seats, by workspace name.</summary>
    Task<ListResultDto<WorkspaceSubscriptionDto>> GetWorkspaceSubscriptionsAsync();

    /// <summary>POST /api/app/platform/change-workspace-plan?workspaceId=&amp;planId= — moves a workspace to another plan. A
    /// free plan makes the subscription active; on a paid plan its status is kept. Refused
    /// (<c>Tracker:SeatLimitReached</c>) when the workspace occupies more seats than the plan allows.</summary>
    Task<SubscriptionDto> ChangeWorkspacePlanAsync(Guid workspaceId, Guid planId);

    /// <summary>POST /api/app/platform/activate-subscription?workspaceId= — marks a workspace's subscription active (ends
    /// a trial, lifts past_due or a suspension).</summary>
    Task<SubscriptionDto> ActivateSubscriptionAsync(Guid workspaceId);

    /// <summary>POST /api/app/platform/suspend-subscription?workspaceId= — suspends a workspace: it can no longer record
    /// time until activated again.</summary>
    Task<SubscriptionDto> SuspendSubscriptionAsync(Guid workspaceId);

    /// <summary>POST /api/app/platform/mark-invoice-paid?invoiceId= — records an invoice as paid. Paying a workspace's
    /// last overdue invoice makes it active again.</summary>
    Task<PlatformInvoiceDto> MarkInvoicePaidAsync(Guid invoiceId);
}
