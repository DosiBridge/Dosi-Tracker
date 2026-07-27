using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Notifications;

public class NotificationTests
{
    [Fact]
    public void New_Notification_Should_Start_Unread()
    {
        var userId = Guid.NewGuid();
        var notification = new Notification(Guid.NewGuid(), null, userId, "Weekly report is ready");

        notification.UserId.ShouldBe(userId);
        notification.Message.ShouldBe("Weekly report is ready");
        notification.IsRead.ShouldBeFalse();
    }
}
