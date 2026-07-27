using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Billing;

public class InvoiceDto : FullAuditedEntityDto<Guid>
{
    public decimal Amount { get; set; }
    public string Status { get; set; }
    public DateTime DueDate { get; set; }
}
