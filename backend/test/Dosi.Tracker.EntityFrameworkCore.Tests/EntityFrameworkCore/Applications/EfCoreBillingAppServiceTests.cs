using Dosi.Tracker.Billing;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreBillingAppServiceTests : BillingAppServiceTests<TrackerEntityFrameworkCoreTestModule>
{

}
