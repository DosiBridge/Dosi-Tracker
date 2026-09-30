using Dosi.Tracker.Account;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreSelfRegistrationTests : SelfRegistrationTests<TrackerEntityFrameworkCoreTestModule>
{

}
