using System;
using Volo.Abp.Data;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Billing;

public class Invoice : FullAuditedAggregateRoot<Guid>, IMultiTenant
{
    public const string PendingStatus = "pending";
    public const string PaidStatus = "paid";
    public const string FailedStatus = "failed";

    /// <summary>Extra property set once the invoice going overdue has been acted on (the workspace marked past_due,
    /// its owners notified), so that happens once per invoice.</summary>
    public const string OverdueNoticeSentProperty = "OverdueNoticeSent";

    /// <summary>Extra property set when the host reactivated the workspace while this invoice was overdue: it is still
    /// owed (and still listed as unpaid), but it no longer holds the workspace past due — it neither shortens the grace
    /// period of a later unpaid invoice nor keeps the workspace suspended once that one is paid.</summary>
    public const string OverdueExcusedProperty = "OverdueExcused";

    public Guid? TenantId { get; protected set; }
    public decimal Amount { get; set; }
    public string Status { get; set; } // paid, pending, failed
    public DateTime DueDate { get; set; }

    /// <summary>UTC start of the billed month; at most one invoice per tenant and period (null on legacy invoices).</summary>
    public DateTime? BillingPeriodStart { get; set; }

    protected Invoice() { }

    public Invoice(Guid id, Guid? tenantId, decimal amount, DateTime dueDate)
        : base(id)
    {
        TenantId = tenantId;
        Amount = amount;
        DueDate = dueDate;
        Status = PendingStatus;
    }

    /// <summary>UTC start of the billing period (the calendar month) that contains <paramref name="moment"/>.</summary>
    public static DateTime GetBillingPeriodStart(DateTime moment)
    {
        return new DateTime(moment.Year, moment.Month, 1, 0, 0, 0, DateTimeKind.Utc);
    }

    public bool IsPaid()
    {
        return Status == PaidStatus;
    }

    /// <summary>Unpaid (pending or failed) and past its due date.</summary>
    public bool IsOverdue(DateTime now)
    {
        return !IsPaid() && DueDate < now;
    }

    public void MarkPaid()
    {
        Status = PaidStatus;
    }

    public bool IsOverdueNoticeSent()
    {
        return this.GetProperty<bool>(OverdueNoticeSentProperty);
    }

    public void MarkOverdueNoticeSent()
    {
        this.SetProperty(OverdueNoticeSentProperty, true);
    }

    public bool IsOverdueExcused()
    {
        return this.GetProperty<bool>(OverdueExcusedProperty);
    }

    /// <summary>See <see cref="OverdueExcusedProperty"/>.</summary>
    public void ExcuseOverdue()
    {
        this.SetProperty(OverdueExcusedProperty, true);
    }

    /// <summary>Overdue (<see cref="IsOverdue"/>) and not excused by the host: an invoice that holds its workspace
    /// past due.</summary>
    public bool HoldsWorkspacePastDue(DateTime now)
    {
        return IsOverdue(now) && !IsOverdueExcused();
    }
}
