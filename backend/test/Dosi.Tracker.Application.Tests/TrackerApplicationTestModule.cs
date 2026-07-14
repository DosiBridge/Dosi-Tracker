using Volo.Abp.Modularity;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerApplicationModule),
    typeof(TrackerDomainTestModule)
)]
public class TrackerApplicationTestModule : AbpModule
{

}
