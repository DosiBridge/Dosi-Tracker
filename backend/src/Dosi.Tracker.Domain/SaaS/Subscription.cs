using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.SaaS;

public class Subscription : FullAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; protected set; }
    public Guid PlanId { get; set; }
    public string Status { get; set; } // active, trialing, past_due
    public DateTime? TrialEndsAt { get; set; }
    
    protected Subscription() { }
    
    public Subscription(Guid id, Guid? tenantId, Guid planId) 
        : base(id)
    {
        TenantId = tenantId;
        PlanId = planId;
        Status = "trialing";
    }
}
