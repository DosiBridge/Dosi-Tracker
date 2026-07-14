using System;
using System.Threading.Tasks;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Dosi.Tracker.Data;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.EntityFrameworkCore;

public class EntityFrameworkCoreTrackerDbSchemaMigrator
    : ITrackerDbSchemaMigrator, ITransientDependency
{
    private readonly IServiceProvider _serviceProvider;

    public EntityFrameworkCoreTrackerDbSchemaMigrator(IServiceProvider serviceProvider)
    {
        _serviceProvider = serviceProvider;
    }

    public async Task MigrateAsync()
    {
        /* We intentionally resolving the TrackerDbContext
         * from IServiceProvider (instead of directly injecting it)
         * to properly get the connection string of the current tenant in the
         * current scope.
         */

        await _serviceProvider
            .GetRequiredService<TrackerDbContext>()
            .Database
            .MigrateAsync();
    }
}
