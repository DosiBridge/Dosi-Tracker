using Dosi.Tracker.Reporting;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreReportingDataQualityTests : ReportingDataQualityTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreReportingDataQualityScopeTests : ReportingDataQualityScopeTests<TrackerEntityFrameworkCoreTestModule>
{
}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreReportingDataQualityWithoutReportingTests
    : ReportingDataQualityWithoutReportingTests<TrackerEntityFrameworkCoreTestModule>
{
}
