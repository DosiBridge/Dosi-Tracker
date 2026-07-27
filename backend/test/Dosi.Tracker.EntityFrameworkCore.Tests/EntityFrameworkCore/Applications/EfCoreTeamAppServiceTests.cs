using Dosi.Tracker.Teams;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreTeamAppServiceTests : TeamAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
