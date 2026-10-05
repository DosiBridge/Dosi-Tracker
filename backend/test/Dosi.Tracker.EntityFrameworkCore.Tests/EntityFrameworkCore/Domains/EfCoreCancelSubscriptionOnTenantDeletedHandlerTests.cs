using Dosi.Tracker.SaaS;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreCancelSubscriptionOnTenantDeletedHandlerTests : CancelSubscriptionOnTenantDeletedHandlerTests<TrackerEntityFrameworkCoreTestModule>
{

}
