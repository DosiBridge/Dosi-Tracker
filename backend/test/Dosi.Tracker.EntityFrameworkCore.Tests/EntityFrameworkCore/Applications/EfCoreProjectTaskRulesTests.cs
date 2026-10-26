using Dosi.Tracker.Projects;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreProjectTaskRulesTests : ProjectTaskRulesTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreProjectTaskRulesMemberTests : ProjectTaskRulesMemberTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreProjectTaskRulesViewerTests : ProjectTaskRulesViewerTests<TrackerEntityFrameworkCoreTestModule>
{
}
