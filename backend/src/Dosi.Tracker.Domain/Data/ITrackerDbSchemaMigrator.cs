using System.Threading.Tasks;

namespace Dosi.Tracker.Data;

public interface ITrackerDbSchemaMigrator
{
    Task MigrateAsync();
}
