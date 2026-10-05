using Dosi.Tracker.Security;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreOpenIddictClientSeedingTests : OpenIddictClientSeedingTests<TrackerEntityFrameworkCoreTestModule>
{

}
