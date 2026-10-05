using Dosi.Tracker.Presence;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCorePresenceAppServiceTests : PresenceAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
