using Dosi.Tracker.AiAssist;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Domains;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiSettingsManagerTests : AiSettingsManagerTests<TrackerEntityFrameworkCoreTestModule>
{
}
