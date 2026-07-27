using Dosi.Tracker.Reporting;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreReportingAppServiceTests : ReportingAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
