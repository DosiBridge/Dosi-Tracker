using Dosi.Tracker.Notifications;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreNotificationAppServiceTests : NotificationAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
