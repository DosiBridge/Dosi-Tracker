using Dosi.Tracker.AiAssist.Features;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

/// <summary>The work helpers (data-quality review, note suggestions, task drafts) and feedback on EF Core / SQLite.</summary>
public static class EfCoreAiWorkFeatureTests
{
    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreDataQualityReviewTests : DataQualityReviewTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreDataQualityReviewWorkerTests : DataQualityReviewWorkerTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreNoteSuggestionsTests : NoteSuggestionsTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreNoteSuggestionsWorkerTests : NoteSuggestionsWorkerTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreTaskDraftsTests : TaskDraftsTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreTaskDraftsClientTests : TaskDraftsClientTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreFeedbackTests : FeedbackTests<TrackerEntityFrameworkCoreTestModule>
    {
    }
}
