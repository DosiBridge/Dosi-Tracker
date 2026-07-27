using Dosi.Tracker.Platform;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCorePlatformAppServiceTests : PlatformAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
