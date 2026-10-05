using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Dosi.Tracker.Identity;
using Dosi.Tracker.MultiTenancy;
using Volo.Abp.TenantManagement;

namespace Dosi.Tracker.Data;

public class TrackerDbMigrationService : ITransientDependency
{
    /// <summary>Email of the platform (host) administrator created by the first DbMigrator run.</summary>
    public const string HostAdminEmailKey = "App:HostAdminEmail";

    /// <summary>Password of the platform (host) administrator created by the first DbMigrator run — set it, e.g. as the
    /// environment variable App__HostAdminPassword. Outside Development that run refuses to create the administrator
    /// without it (or with the template password); in Development the template password is used and must be changed at
    /// the first sign-in.</summary>
    public const string HostAdminPasswordKey = "App:HostAdminPassword";

    public ILogger<TrackerDbMigrationService> Logger { get; set; }

    private readonly IDataSeeder _dataSeeder;
    private readonly IEnumerable<ITrackerDbSchemaMigrator> _dbSchemaMigrators;
    private readonly ITenantRepository _tenantRepository;
    private readonly ICurrentTenant _currentTenant;
    private readonly IConfiguration _configuration;
    private readonly IAbpHostEnvironment _hostEnvironment;
    private readonly TrackerIdentityDataSeeder _identityDataSeeder;

    public TrackerDbMigrationService(
        IDataSeeder dataSeeder,
        ITenantRepository tenantRepository,
        ICurrentTenant currentTenant,
        IEnumerable<ITrackerDbSchemaMigrator> dbSchemaMigrators,
        IConfiguration configuration,
        IAbpHostEnvironment hostEnvironment,
        TrackerIdentityDataSeeder identityDataSeeder)
    {
        _dataSeeder = dataSeeder;
        _tenantRepository = tenantRepository;
        _currentTenant = currentTenant;
        _dbSchemaMigrators = dbSchemaMigrators;
        _configuration = configuration;
        _hostEnvironment = hostEnvironment;
        _identityDataSeeder = identityDataSeeder;

        Logger = NullLogger<TrackerDbMigrationService>.Instance;
    }

    public async Task MigrateAsync()
    {
        var initialMigrationAdded = AddInitialMigrationIfNotExist();

        if (initialMigrationAdded)
        {
            return;
        }

        Logger.LogInformation("Started database migrations...");

        await MigrateDatabaseSchemaAsync();
        await SeedDataAsync();

        Logger.LogInformation($"Successfully completed host database migrations.");

        if (MultiTenancyConsts.IsEnabled)
        {
            
            var tenants = await _tenantRepository.GetListAsync(includeDetails: true);

            var migratedDatabaseSchemas = new HashSet<string>();
            foreach (var tenant in tenants)
            {
                using (_currentTenant.Change(tenant.Id))
                {
                    if (tenant.ConnectionStrings.Any())
                    {
                        var tenantConnectionStrings = tenant.ConnectionStrings
                            .Select(x => x.Value)
                            .ToList();

                        if (!migratedDatabaseSchemas.IsSupersetOf(tenantConnectionStrings))
                        {
                            await MigrateDatabaseSchemaAsync(tenant);

                            migratedDatabaseSchemas.AddIfNotContains(tenantConnectionStrings);
                        }
                    }

                    await SeedDataAsync(tenant);
                }

                Logger.LogInformation($"Successfully completed {tenant.Name} tenant database migrations.");
            }

            Logger.LogInformation("Successfully completed all database migrations.");
        }
        Logger.LogInformation("You can safely end this process...");
    }

    private async Task MigrateDatabaseSchemaAsync(Tenant? tenant = null)
    {
        Logger.LogInformation(
            $"Migrating schema for {(tenant == null ? "host" : tenant.Name + " tenant")} database...");
        
        foreach (var migrator in _dbSchemaMigrators)
        {
            await migrator.MigrateAsync();
        }
    }

    private async Task SeedDataAsync(Tenant? tenant = null)
    {
        Logger.LogInformation($"Executing {(tenant == null ? "host" : tenant.Name + " tenant")} database seed...");

        if (tenant == null)
        {
            await SeedHostAsync();
        }
        else
        {
            await SeedTenantAsync(tenant);
        }
    }

    /// <summary>
    /// Seeds the host. Its administrator is created only on the first run (a host without any user, see
    /// <see cref="TrackerIdentityDataSeeder"/>), with the credentials of <see cref="CreateHostSeedContext"/>. That first
    /// run refuses, outside Development, to create the administrator with the template password (none configured, or
    /// <c>1q2w3E*</c> itself): the platform owner holds every workspace's users, invoices and tenant administration, and
    /// a fresh deployment must not open with a password anyone can look up. Later runs are not affected.
    /// </summary>
    public virtual async Task SeedHostAsync()
    {
        var context = CreateHostSeedContext();
        await EnsureSafeHostAdminPasswordAsync(context);
        await _dataSeeder.SeedAsync(context);
    }

    /// <summary>Throws when this run would create the first host administrator with the template password outside
    /// Development (see <see cref="SeedHostAsync"/>).</summary>
    protected virtual async Task EnsureSafeHostAdminPasswordAsync(DataSeedContext context)
    {
        var password = context[IdentityDataSeedContributor.AdminPasswordPropertyName] as string;
        if (_hostEnvironment.IsDevelopment() ||
            !TrackerIdentityDataSeeder.IsTemplatePassword(password) ||
            await _identityDataSeeder.HasAnyUserAsync(null))
        {
            return;
        }

        throw new AbpException(
            $"Refusing to create the first platform (host) administrator with the well-known template password in the " +
            $"'{_hostEnvironment.EnvironmentName}' environment. Set {HostAdminPasswordKey} to a strong password for this first " +
            "DbMigrator run (e.g. the environment variable App__HostAdminPassword), or run it with " +
            "DOTNET_ENVIRONMENT=Development for a local database.");
    }

    /// <summary>
    /// Seeds an existing workspace (roles, permissions, …) without ever creating an administrator: the owner was created
    /// together with the workspace, and re-creating a template <c>admin</c> account in a workspace whose owner renamed
    /// or removed it would hand out full owner rights with a well-known password.
    /// </summary>
    public virtual async Task SeedTenantAsync(Tenant tenant)
    {
        using (TrackerIdentityDataSeeder.SuppressAdminCreation())
        {
            await _dataSeeder.SeedAsync(new DataSeedContext(tenant.Id));
        }
    }

    /// <summary>The host seed context: the first platform administrator's email and password come from
    /// <see cref="HostAdminEmailKey"/> / <see cref="HostAdminPasswordKey"/>, falling back to the template values (which
    /// <see cref="SeedHostAsync"/> accepts for a new host in Development only).</summary>
    public virtual DataSeedContext CreateHostSeedContext()
    {
        var email = _configuration[HostAdminEmailKey];
        var password = _configuration[HostAdminPasswordKey];

        return new DataSeedContext()
            .WithProperty(IdentityDataSeedContributor.AdminEmailPropertyName,
                string.IsNullOrWhiteSpace(email) ? TrackerConsts.AdminEmailDefaultValue : email.Trim())
            .WithProperty(IdentityDataSeedContributor.AdminPasswordPropertyName,
                string.IsNullOrWhiteSpace(password) ? TrackerConsts.AdminPasswordDefaultValue : password);
    }

    private bool AddInitialMigrationIfNotExist()
    {
        try
        {
            if (!DbMigrationsProjectExists())
            {
                return false;
            }
        }
        catch (Exception)
        {
            return false;
        }

        try
        {
            if (!MigrationsFolderExists())
            {
                AddInitialMigration();
                return true;
            }
            else
            {
                return false;
            }
        }
        catch (Exception e)
        {
            Logger.LogWarning("Couldn't determinate if any migrations exist : " + e.Message);
            return false;
        }
    }

    private bool DbMigrationsProjectExists()
    {
        var dbMigrationsProjectFolder = GetEntityFrameworkCoreProjectFolderPath();

        return dbMigrationsProjectFolder != null;
    }

    private bool MigrationsFolderExists()
    {
        var dbMigrationsProjectFolder = GetEntityFrameworkCoreProjectFolderPath();

        return dbMigrationsProjectFolder != null && Directory.Exists(Path.Combine(dbMigrationsProjectFolder, "Migrations"));
    }

    private void AddInitialMigration()
    {
        Logger.LogInformation("Creating initial migration...");

        string argumentPrefix;
        string fileName;

        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX) || RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
        {
            argumentPrefix = "-c";
            fileName = "/bin/bash";
        }
        else
        {
            argumentPrefix = "/C";
            fileName = "cmd.exe";
        }

        var procStartInfo = new ProcessStartInfo(fileName,
            $"{argumentPrefix} \"abp create-migration-and-run-migrator \"{GetEntityFrameworkCoreProjectFolderPath()}\"\""
        );

        try
        {
            Process.Start(procStartInfo);
        }
        catch (Exception)
        {
            throw new Exception("Couldn't run ABP CLI...");
        }
    }

    private string? GetEntityFrameworkCoreProjectFolderPath()
    {
        var slnDirectoryPath = GetSolutionDirectoryPath();

        if (slnDirectoryPath == null)
        {
            throw new Exception("Solution folder not found!");
        }

        var srcDirectoryPath = Path.Combine(slnDirectoryPath, "src");

        return Directory.GetDirectories(srcDirectoryPath)
            .FirstOrDefault(d => d.EndsWith(".EntityFrameworkCore"));
    }

    private string? GetSolutionDirectoryPath()
    {
        var currentDirectory = new DirectoryInfo(Directory.GetCurrentDirectory());

        while (currentDirectory != null && Directory.GetParent(currentDirectory.FullName) != null)
        {
            currentDirectory = Directory.GetParent(currentDirectory.FullName);

            if (currentDirectory != null && Directory.GetFiles(currentDirectory.FullName).FirstOrDefault(f => f.EndsWith(".sln") || f.EndsWith(".slnx")) != null)
            {
                return currentDirectory.FullName;
            }
        }

        return null;
    }
}
