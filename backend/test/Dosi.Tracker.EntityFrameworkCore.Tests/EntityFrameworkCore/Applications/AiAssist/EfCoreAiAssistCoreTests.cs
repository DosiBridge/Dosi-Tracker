using Dosi.Tracker.AiAssist;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiAccessGuardTests : AiAccessGuardTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiStatusTests : AiStatusTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiStatusWithoutProviderTests : AiStatusWithoutProviderTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiSettingsAppServiceTests : AiSettingsAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCorePlatformAiAppServiceTests : PlatformAiAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{
}
