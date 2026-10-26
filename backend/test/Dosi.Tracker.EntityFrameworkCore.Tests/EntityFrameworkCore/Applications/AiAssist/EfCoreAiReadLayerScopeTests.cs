using Dosi.Tracker.AiAssist.Read;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiReadLayerOwnerScopeTests : AiReadLayerOwnerScopeTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiReadLayerManagerWithoutReportingScopeTests
    : AiReadLayerManagerWithoutReportingScopeTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiReadLayerWorkerScopeTests : AiReadLayerWorkerScopeTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiReadLayerClientWorkerScopeTests : AiReadLayerClientWorkerScopeTests<TrackerEntityFrameworkCoreTestModule>
{
}
