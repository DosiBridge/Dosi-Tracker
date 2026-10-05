using Dosi.Tracker.Identity;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreHostAdministratorBootstrapTests : HostAdministratorBootstrapTests<TrackerEntityFrameworkCoreTestModule>
{

}

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreDevelopmentHostAdministratorBootstrapTests : DevelopmentHostAdministratorBootstrapTests<TrackerEntityFrameworkCoreTestModule>
{

}
