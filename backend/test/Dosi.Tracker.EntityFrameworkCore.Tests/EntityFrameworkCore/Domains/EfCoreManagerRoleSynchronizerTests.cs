using Dosi.Tracker.Identity;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreManagerRoleSynchronizerTests : ManagerRoleSynchronizerTests<TrackerEntityFrameworkCoreTestModule>
{

}
