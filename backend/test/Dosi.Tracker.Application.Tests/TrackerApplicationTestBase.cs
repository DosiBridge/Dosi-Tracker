using Volo.Abp.Modularity;

namespace Dosi.Tracker;

public abstract class TrackerApplicationTestBase<TStartupModule> : TrackerTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{

}
