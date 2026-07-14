using System;
using System.IO;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Design;
using Microsoft.Extensions.Configuration;

namespace Dosi.Tracker.EntityFrameworkCore;

/* This class is needed for EF Core console commands
 * (like Add-Migration and Update-Database commands) */
public class TrackerDbContextFactory : IDesignTimeDbContextFactory<TrackerDbContext>
{
    public TrackerDbContext CreateDbContext(string[] args)
    {
        // https://www.npgsql.org/efcore/release-notes/6.0.html#opting-out-of-the-new-timestamp-mapping-logic
        AppContext.SetSwitch("Npgsql.EnableLegacyTimestampBehavior", true);
        
        var configuration = BuildConfiguration();
        
        TrackerEfCoreEntityExtensionMappings.Configure();

        var builder = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql(configuration.GetConnectionString("Default"));
        
        return new TrackerDbContext(builder.Options);
    }

    private static IConfigurationRoot BuildConfiguration()
    {
        var builder = new ConfigurationBuilder()
            .SetBasePath(Path.Combine(Directory.GetCurrentDirectory(), "../Dosi.Tracker.DbMigrator/"))
            .AddJsonFile("appsettings.json", optional: false)
            .AddEnvironmentVariables();

        return builder.Build();
    }
}
