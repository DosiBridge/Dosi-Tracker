using System;
using System.ComponentModel.DataAnnotations;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Billing;

/// <summary>The current workspace's billing (workspace owners: <c>Tracker.Billing</c>).</summary>
public interface IBillingAppService : IApplicationService
{
    Task<PagedResultDto<InvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input);

    /// <summary><c>POST /api/app/billing/change-plan</c>: moves the workspace to a free plan whose seat limit its team
    /// fits (<c>Tracker:SeatLimitReached</c>); a paid plan is assigned by the platform (<c>Tracker:PlanChangeNotAllowed</c>).
    /// A canceled or lapsed workspace is active again on the free plan. <c>Tracker:BillingDisabled</c> while this
    /// deployment does not bill.</summary>
    Task<SubscriptionDto> ChangePlanAsync(ChangePlanDto input);

    /// <summary><c>POST /api/app/billing/cancel</c>: cancels the paid subscription — no more invoices, and time tracking
    /// pauses until the workspace moves to a free plan or the platform activates it again. Invoices already issued stay
    /// owed. <c>Tracker:PlanChangeNotAllowed</c> on a free plan, <c>Tracker:BillingDisabled</c> while this deployment
    /// does not bill.</summary>
    Task<SubscriptionDto> CancelAsync();
}

public class ChangePlanDto
{
    [Required]
    public Guid PlanId { get; set; }
}
