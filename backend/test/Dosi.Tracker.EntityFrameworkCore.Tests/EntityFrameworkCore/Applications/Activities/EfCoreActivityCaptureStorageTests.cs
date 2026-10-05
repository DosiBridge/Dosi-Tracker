using Dosi.Tracker.Activities;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreActivityCaptureStorageTests : ActivityCaptureStorageTests<TrackerEntityFrameworkCoreTestModule>
{

}
