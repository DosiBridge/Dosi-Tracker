using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Notifications;

public class Notification : CreationAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; protected set; }
    public Guid UserId { get; set; }
    public string Message { get; set; }
    public bool IsRead { get; set; }
    
    protected Notification() { }
    
    public Notification(Guid id, Guid? tenantId, Guid userId, string message) 
        : base(id)
    {
        TenantId = tenantId;
        UserId = userId;
        Message = message;
        IsRead = false;
    }
}
