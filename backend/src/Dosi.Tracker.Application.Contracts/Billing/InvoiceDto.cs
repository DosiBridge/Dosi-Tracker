using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Billing;

public class InvoiceDto : FullAuditedEntityDto<Guid>
{
    public decimal Amount { get; set; }
    public string Status { get; set; }
    public DateTime DueDate { get; set; }

    /// <summary>UTC start of the billed month (unique per tenant); null on invoices from before billing periods.</summary>
    public DateTime? BillingPeriodStart { get; set; }
}
