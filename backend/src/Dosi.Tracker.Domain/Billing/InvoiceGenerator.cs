using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;
using Volo.Abp.Timing;

namespace Dosi.Tracker.Billing;

/// <summary>
/// Generates the monthly per-seat invoice for a subscription.
/// A "seat" is a distinct user with at least one project membership in the tenant
/// (owner/admins are members of the projects they created).
/// </summary>
public class InvoiceGenerator : DomainService
{
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Plan, Guid> _planRepository;

    public InvoiceGenerator(
        IRepository<Invoice, Guid> invoiceRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Plan, Guid> planRepository)
    {
        _invoiceRepository = invoiceRepository;
        _memberRepository = memberRepository;
        _planRepository = planRepository;
    }

    /// <summary>Creates this month's invoice for the (current-tenant) subscription
    /// unless one already exists or the plan is free. Returns the invoice, or null.</summary>
    public async Task<Invoice?> GenerateForCurrentMonthAsync(Subscription subscription)
    {
        var plan = await _planRepository.FindAsync(subscription.PlanId);
        if (plan == null || plan.PricePerUser <= 0m)
        {
            return null; // Unknown or free plan — nothing to bill.
        }

        var monthStart = new DateTime(Clock.Now.Year, Clock.Now.Month, 1, 0, 0, 0, DateTimeKind.Utc);
        var alreadyInvoiced = await _invoiceRepository.AnyAsync(
            i => i.TenantId == subscription.TenantId && i.CreationTime >= monthStart);
        if (alreadyInvoiced)
        {
            return null;
        }

        var seatCount = await CountSeatsAsync();
        if (seatCount == 0)
        {
            seatCount = 1; // The workspace admin always occupies a seat.
        }

        var invoice = new Invoice(
            GuidGenerator.Create(),
            subscription.TenantId,
            plan.PricePerUser * seatCount,
            Clock.Now.AddDays(14));

        await _invoiceRepository.InsertAsync(invoice);
        return invoice;
    }

    public async Task<int> CountSeatsAsync()
    {
        var memberQuery = await _memberRepository.GetQueryableAsync();
        return await AsyncExecuter.CountAsync(
            memberQuery.Select(m => m.UserId).Distinct());
    }
}
