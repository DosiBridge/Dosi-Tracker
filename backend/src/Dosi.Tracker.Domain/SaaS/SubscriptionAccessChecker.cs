using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker.SaaS;

/// <inheritdoc cref="ISubscriptionAccessChecker"/>
public class SubscriptionAccessChecker : DomainService, ISubscriptionAccessChecker
{
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;

    public SubscriptionAccessChecker(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Invoice, Guid> invoiceRepository)
    {
        _subscriptionRepository = subscriptionRepository;
        _invoiceRepository = invoiceRepository;
    }

    public virtual async Task<bool> IsUsableAsync()
    {
        if (CurrentTenant.Id == null)
        {
            return true; // The host (platform) has no subscription of its own.
        }

        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.TenantId == CurrentTenant.Id);
        if (subscription == null)
        {
            return true; // Nothing to enforce, as for seat limits.
        }

        var now = Clock.Now;
        switch (subscription.Status)
        {
            case Subscription.ActiveStatus:
                return true;
            case Subscription.TrialingStatus:
                // Same cut-off as TrialExpiryWorker (over once TrialEndsAt < now); a trial without an end never ends.
                return !subscription.IsTrialOver(now);
            case Subscription.PastDueStatus:
                var graceEndsAt = await GetPastDueGraceEndAsync();
                return graceEndsAt.HasValue && now < graceEndsAt.Value;
            default:
                return false; // canceled, expired, suspended or unknown
        }
    }

    /// <summary>
    /// When a past_due workspace of the current tenant stops being usable:
    /// <see cref="Subscription.PastDueGraceDays"/> days after its oldest unpaid invoice fell due. Invoices the host
    /// excused by reactivating the workspace (<see cref="Invoice.IsOverdueExcused"/>) do not count: a later unpaid
    /// invoice gets its full grace period. Null when nothing (else) is unpaid — a workspace whose trial ended has no
    /// invoice and therefore no grace period.
    /// </summary>
    public virtual async Task<DateTime?> GetPastDueGraceEndAsync()
    {
        // A workspace has few unpaid invoices; the excused flag is an extra property, so it is filtered here.
        var unpaid = await _invoiceRepository.GetListAsync(
            i => i.TenantId == CurrentTenant.Id && i.Status != Invoice.PaidStatus);
        var oldestDueDate = unpaid
            .Where(i => !i.IsOverdueExcused())
            .Select(i => (DateTime?)i.DueDate)
            .Min();

        return oldestDueDate?.AddDays(Subscription.PastDueGraceDays);
    }
}
