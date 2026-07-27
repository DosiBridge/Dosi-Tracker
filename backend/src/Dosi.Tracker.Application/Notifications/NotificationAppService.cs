using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Notifications;

[Authorize]
public class NotificationAppService : TrackerAppService, INotificationAppService
{
    private readonly IRepository<Notification, Guid> _notificationRepository;

    public NotificationAppService(IRepository<Notification, Guid> notificationRepository)
    {
        _notificationRepository = notificationRepository;
    }

    public async Task<ListResultDto<NotificationDto>> GetMyNotificationsAsync()
    {
        var userId = CurrentUser.GetId();
        var notifications = await _notificationRepository.GetListAsync(n => n.UserId == userId);

        return new ListResultDto<NotificationDto>(
            ObjectMapper.Map<List<Notification>, List<NotificationDto>>(notifications)
        );
    }

    public async Task MarkAsReadAsync(Guid notificationId)
    {
        var userId = CurrentUser.GetId();
        var notification = await _notificationRepository.FirstOrDefaultAsync(
            n => n.Id == notificationId && n.UserId == userId);

        // Only the owner may mark a notification as read; hide others' notifications entirely.
        if (notification == null)
        {
            throw new EntityNotFoundException(typeof(Notification), notificationId);
        }

        notification.IsRead = true;
        await _notificationRepository.UpdateAsync(notification);
    }
}
