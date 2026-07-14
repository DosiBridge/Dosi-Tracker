using System.Threading.Tasks;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.Data;

/* This is used if database provider does't define
 * ITrackerDbSchemaMigrator implementation.
 */
public class NullTrackerDbSchemaMigrator : ITrackerDbSchemaMigrator, ITransientDependency
{
    public Task MigrateAsync()
    {
        return Task.CompletedTask;
    }
}
