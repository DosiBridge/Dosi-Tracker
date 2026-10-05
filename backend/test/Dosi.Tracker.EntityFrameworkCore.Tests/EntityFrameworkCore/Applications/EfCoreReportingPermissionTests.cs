using Dosi.Tracker.Reporting;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreReportingPermissionTests : ReportingPermissionTests<TrackerEntityFrameworkCoreTestModule>
{

}
