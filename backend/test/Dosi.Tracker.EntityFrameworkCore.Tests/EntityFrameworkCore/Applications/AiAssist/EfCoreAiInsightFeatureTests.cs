using Dosi.Tracker.AiAssist.Features;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

// AI feature tests sign in as the same member and share the process-wide per-member AI lock: they run one after
// another in the shared collection.
/// <summary>The insight cards (period summary, member day, report explanation) on EF Core / SQLite.</summary>
public static class EfCoreAiInsightFeatureTests
{
    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCorePeriodSummaryTests : PeriodSummaryTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCorePeriodSummaryWorkerTests : PeriodSummaryWorkerTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCorePeriodSummaryManagerWithoutReportingTests : PeriodSummaryManagerWithoutReportingTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreMemberDaySummaryTests : MemberDaySummaryTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreMemberDaySummaryWorkerTests : MemberDaySummaryWorkerTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreReportExplanationTests : ReportExplanationTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreReportExplanationMemberTests : ReportExplanationMemberTests<TrackerEntityFrameworkCoreTestModule>
    {
    }
}
