using Dosi.Tracker.SaaS;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreWorkspaceAppServiceTests : WorkspaceAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
