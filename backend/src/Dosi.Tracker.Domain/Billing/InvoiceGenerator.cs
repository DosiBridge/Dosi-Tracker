using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker.Billing;

/// <summary>
/// Generates the monthly per-seat invoice for a subscription: the plan's per-user price times the workspace's
/// occupied seats (<see cref="SeatCounter"/>; at least one, the owner always occupies a seat), due
/// <see cref="PaymentTermDays"/> days later. A workspace gets at most one invoice per billing period (calendar
/// month, <see cref="Invoice.BillingPeriodStart"/>); a unique index on (tenant, period) backs this up when two
/// sweeps race.
/// </summary>
public class InvoiceGenerator : DomainService
{
    /// <summary>Days between generating an invoice and its due date.</summary>
    public const int PaymentTermDays = 14;

    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly SeatCounter _seatCounter;

    public InvoiceGenerator(
        IRepository<Invoice, Guid> invoiceRepository,
        IRepository<Plan, Guid> planRepository,
        SeatCounter seatCounter)
    {
        _invoiceRepository = invoiceRepository;
        _planRepository = planRepository;
        _seatCounter = seatCounter;
    }

    /// <summary>Creates this month's invoice for the (current-tenant) subscription
    /// unless one already exists or the plan is free. Returns the invoice, or null.</summary>
    public virtual Task<Invoice?> GenerateForCurrentMonthAsync(Subscription subscription)
    {
        return GenerateForPeriodAsync(subscription, Clock.Now);
    }

    /// <summary>Creates the invoice of the billing period containing <paramref name="periodStart"/> for the
    /// (current-tenant) subscription unless the period is already invoiced or the plan is free or unknown.
    /// Returns the invoice (saved with the caller's unit of work), or null.</summary>
    public virtual async Task<Invoice?> GenerateForPeriodAsync(Subscription subscription, DateTime periodStart)
    {
        var plan = await _planRepository.FindAsync(subscription.PlanId);
        if (plan == null || plan.IsFree())
        {
            return null; // Unknown or free plan — nothing to bill.
        }

        var billingPeriodStart = Invoice.GetBillingPeriodStart(periodStart);
        if (await IsInvoicedAsync(subscription.TenantId, billingPeriodStart))
        {
            return null;
        }

        var seatCount = Math.Max(1, await _seatCounter.CountAsync()); // The workspace owner always occupies a seat.

        var invoice = new Invoice(
            GuidGenerator.Create(),
            subscription.TenantId,
            plan.PricePerUser * seatCount,
            Clock.Now.AddDays(PaymentTermDays))
        {
            BillingPeriodStart = billingPeriodStart
        };

        await _invoiceRepository.InsertAsync(invoice);
        return invoice;
    }

    /// <summary>True when the workspace already has an invoice for the billing period containing
    /// <paramref name="periodStart"/>, counting a legacy invoice (no period) created during that month.</summary>
    public virtual async Task<bool> IsInvoicedAsync(Guid? tenantId, DateTime periodStart)
    {
        var start = Invoice.GetBillingPeriodStart(periodStart);
        var end = start.AddMonths(1);
        return await _invoiceRepository.AnyAsync(i =>
            i.TenantId == tenantId &&
            (i.BillingPeriodStart == start ||
             (i.BillingPeriodStart == null && i.CreationTime >= start && i.CreationTime < end)));
    }

    /// <summary>Occupied seats of the current workspace (see <see cref="SeatCounter"/>).</summary>
    public virtual Task<int> CountSeatsAsync()
    {
        return _seatCounter.CountAsync();
    }
}
