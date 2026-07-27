using System;
using System.Linq;
using System.Threading.Tasks;
using Shouldly;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Notifications;

public abstract class NotificationAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly INotificationAppService _notificationAppService;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected NotificationAppServiceTests()
    {
        _notificationAppService = GetRequiredService<INotificationAppService>();
        _notificationRepository = GetRequiredService<IRepository<Notification, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private async Task<Notification> SeedNotificationAsync(Guid userId, string message)
    {
        return await WithUnitOfWorkAsync(async () =>
            await _notificationRepository.InsertAsync(
                new Notification(_guidGenerator.Create(), null, userId, message)));
    }

    [Fact]
    public async Task GetMyNotifications_Should_Only_Return_Current_Users()
    {
        await SeedNotificationAsync(CurrentUserId, "Yours");
        await SeedNotificationAsync(Guid.NewGuid(), "Someone else's");

        var result = await _notificationAppService.GetMyNotificationsAsync();

        result.Items.Count.ShouldBe(1);
        result.Items[0].Message.ShouldBe("Yours");
        result.Items[0].UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task MarkAsRead_Should_Set_IsRead_For_Own_Notification()
    {
        var notification = await SeedNotificationAsync(CurrentUserId, "Weekly report ready");

        await _notificationAppService.MarkAsReadAsync(notification.Id);

        await WithUnitOfWorkAsync(async () =>
        {
            var reloaded = await _notificationRepository.GetAsync(notification.Id);
            reloaded.IsRead.ShouldBeTrue();
        });
    }

    [Fact]
    public async Task MarkAsRead_Should_Not_Touch_Other_Users_Notifications()
    {
        // IDOR guard: marking someone else's notification must behave as "not found".
        var foreign = await SeedNotificationAsync(Guid.NewGuid(), "Not yours");

        await Should.ThrowAsync<EntityNotFoundException>(
            () => _notificationAppService.MarkAsReadAsync(foreign.Id));

        await WithUnitOfWorkAsync(async () =>
        {
            var reloaded = await _notificationRepository.GetAsync(foreign.Id);
            reloaded.IsRead.ShouldBeFalse();
        });
    }
}
