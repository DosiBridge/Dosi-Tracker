using Dosi.Tracker.Projects;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreProjectAppServiceTests : ProjectAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
