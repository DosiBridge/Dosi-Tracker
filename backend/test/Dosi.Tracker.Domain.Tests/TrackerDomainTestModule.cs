using Volo.Abp.Modularity;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerDomainModule),
    typeof(TrackerTestBaseModule)
)]
public class TrackerDomainTestModule : AbpModule
{

}
