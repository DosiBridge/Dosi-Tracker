using Dosi.Tracker.Settings;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreWorkspaceSettingsManagerTests : WorkspaceSettingsManagerTests<TrackerEntityFrameworkCoreTestModule>
{

}
