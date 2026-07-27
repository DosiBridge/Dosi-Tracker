using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Billing;

public class Invoice : FullAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; protected set; }
    public decimal Amount { get; set; }
    public string Status { get; set; } // paid, pending, failed
    public DateTime DueDate { get; set; }
    
    protected Invoice() { }
    
    public Invoice(Guid id, Guid? tenantId, decimal amount, DateTime dueDate) 
        : base(id)
    {
        TenantId = tenantId;
        Amount = amount;
        DueDate = dueDate;
        Status = "pending";
    }
}
