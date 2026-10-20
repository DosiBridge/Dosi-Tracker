using Dosi.Tracker.Security;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreCrossWorkspaceIsolationTests : CrossWorkspaceIsolationTests<TrackerEntityFrameworkCoreTestModule>
{

}
