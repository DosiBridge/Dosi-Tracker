using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore;

[CollectionDefinition(TrackerTestConsts.CollectionDefinitionName)]
public class TrackerEntityFrameworkCoreCollection : ICollectionFixture<TrackerEntityFrameworkCoreFixture>
{

}
