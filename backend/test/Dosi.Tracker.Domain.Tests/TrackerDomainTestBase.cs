using Volo.Abp.Modularity;

namespace Dosi.Tracker;

/* Inherit from this class for your domain layer tests. */
public abstract class TrackerDomainTestBase<TStartupModule> : TrackerTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{

}
