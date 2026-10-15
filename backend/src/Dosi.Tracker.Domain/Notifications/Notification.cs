using System;
using Volo.Abp.Domain.Entities.Auditing;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Notifications;

public class Notification : CreationAuditedAggregateRoot<Guid>, IWorkspaceScoped
{
    public Guid WorkspaceId { get; protected set; }
    public Guid UserId { get; set; }
    public string Message { get; set; }
    public bool IsRead { get; set; }
    
    protected Notification() { }
    
    public Notification(Guid id, Guid workspaceId, Guid userId, string message) 
        : base(id)
    {
        WorkspaceId = workspaceId;
        UserId = userId;
        Message = message;
        IsRead = false;
    }
}
