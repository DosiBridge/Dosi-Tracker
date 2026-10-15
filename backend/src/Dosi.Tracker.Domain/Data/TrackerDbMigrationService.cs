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
using Dosi.Tracker.Identity;

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
    private readonly IConfiguration _configuration;
    private readonly IAbpHostEnvironment _hostEnvironment;
    private readonly TrackerIdentityDataSeeder _identityDataSeeder;

    public TrackerDbMigrationService(
        IDataSeeder dataSeeder,
        IEnumerable<ITrackerDbSchemaMigrator> dbSchemaMigrators,
        IConfiguration configuration,
        IAbpHostEnvironment hostEnvironment,
        TrackerIdentityDataSeeder identityDataSeeder)
    {
        _dataSeeder = dataSeeder;
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

        foreach (var migrator in _dbSchemaMigrators)
        {
            await migrator.MigrateAsync();
        }

        Logger.LogInformation("Executing database seed...");
        await SeedHostAsync();

        Logger.LogInformation("Successfully completed database migrations. You can safely end this process...");
    }

    /// <summary>
    /// Seeds the database: plans, the platform-admin role and its permissions, OpenIddict clients and — on the first run
    /// only — the platform administrator, with the credentials of <see cref="CreateHostSeedContext"/>. Outside
    /// Development that first run refuses to create the administrator with the template password.
    /// </summary>
    public virtual async Task SeedHostAsync()
    {
        var context = CreateHostSeedContext();
        await EnsureSafeHostAdminPasswordAsync(context);
        await _dataSeeder.SeedAsync(context);
    }

    protected virtual async Task EnsureSafeHostAdminPasswordAsync(DataSeedContext context)
    {
        var password = context[IdentityDataSeedContributor.AdminPasswordPropertyName] as string;
        if (_hostEnvironment.IsDevelopment() ||
            !TrackerIdentityDataSeeder.IsTemplatePassword(password) ||
            await _identityDataSeeder.HasAnyUserAsync())
        {
            return;
        }

        throw new AbpException(
            $"Refusing to create the first platform administrator with the well-known template password in the " +
            $"'{_hostEnvironment.EnvironmentName}' environment. Set {HostAdminPasswordKey} to a strong password for this first " +
            "DbMigrator run (e.g. the environment variable App__HostAdminPassword), or run it with " +
            "DOTNET_ENVIRONMENT=Development for a local database.");
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
