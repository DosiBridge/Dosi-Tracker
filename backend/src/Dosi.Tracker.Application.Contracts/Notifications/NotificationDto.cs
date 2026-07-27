using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Notifications;

public class NotificationDto : CreationAuditedEntityDto<Guid>
{
    public Guid UserId { get; set; }
    public string Message { get; set; }
    public bool IsRead { get; set; }
}
