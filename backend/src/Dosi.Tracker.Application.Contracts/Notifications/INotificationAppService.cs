using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Notifications;

public interface INotificationAppService : IApplicationService
{
    Task<ListResultDto<NotificationDto>> GetMyNotificationsAsync();
    Task MarkAsReadAsync(Guid notificationId);
}
