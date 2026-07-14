using Dosi.Tracker.EntityFrameworkCore;
using Volo.Abp.Autofac;
using Volo.Abp.Modularity;

namespace Dosi.Tracker.DbMigrator;

[DependsOn(
    typeof(AbpAutofacModule),
    typeof(TrackerEntityFrameworkCoreModule),
    typeof(TrackerApplicationContractsModule)
)]
public class TrackerDbMigratorModule : AbpModule
{
}
