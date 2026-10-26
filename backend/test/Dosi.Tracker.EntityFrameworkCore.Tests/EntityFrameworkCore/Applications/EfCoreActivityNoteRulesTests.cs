using Dosi.Tracker.Activities;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreActivityNoteRulesTests : ActivityNoteRulesTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreActivityNoteRulesMemberTests : ActivityNoteRulesMemberTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreActivityNoteRulesManagerWithoutViewAllTests
    : ActivityNoteRulesManagerWithoutViewAllTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreActivityNoteRulesViewerTests : ActivityNoteRulesViewerTests<TrackerEntityFrameworkCoreTestModule>
{
}
