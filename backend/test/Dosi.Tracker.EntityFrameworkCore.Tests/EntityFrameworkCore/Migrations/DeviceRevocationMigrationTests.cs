using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Migrations;
using Dosi.Tracker.Workspaces;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Infrastructure;
using Microsoft.EntityFrameworkCore.Migrations;
using Microsoft.EntityFrameworkCore.Migrations.Operations;
using Shouldly;
using Volo.Abp.DependencyInjection;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Migrations;

/// <summary>
/// Device revocation schema migration tests:
/// 1. AddDeviceRevocations migration metadata, Up/Down operations, and model snapshot synchronization.
/// 2. Production PostgreSQL DDL generation (CREATE TABLE "AppDeviceRevocations" with composite unique index).
/// 3. Tracked baseline-to-current upgrade regression preserving existing workspaces, members, and data,
///    verifying successful creation and querying of DeviceRevocation entities post-upgrade.
/// </summary>
public class DeviceRevocationMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private static IReadOnlyList<MigrationOperation> GetUpOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddDeviceRevocations();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddDeviceRevocations)
            .GetMethod("Up", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    private static IReadOnlyList<MigrationOperation> GetDownOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddDeviceRevocations();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddDeviceRevocations)
            .GetMethod("Down", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    [Fact]
    public void Migration_Metadata_And_Attributes_Are_Correctly_Configured()
    {
        var migrationType = typeof(AddDeviceRevocations);

        var migrationAttr = migrationType.GetCustomAttribute<MigrationAttribute>();
        migrationAttr.ShouldNotBeNull();
        migrationAttr.Id.ShouldEndWith("_AddDeviceRevocations");

        var dbContextAttr = migrationType.GetCustomAttribute<DbContextAttribute>();
        dbContextAttr.ShouldNotBeNull();
        dbContextAttr.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Operations_Create_AppDeviceRevocations_Table_And_Unique_Index()
    {
        var operations = GetUpOperations();

        var createTable = operations.OfType<CreateTableOperation>().FirstOrDefault(t => t.Name == "AppDeviceRevocations");
        createTable.ShouldNotBeNull();

        createTable.Columns.ShouldContain(c => c.Name == "Id" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "WorkspaceId" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "UserId" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "DeviceId" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "Reason" && c.ClrType == typeof(string) && c.IsNullable && c.MaxLength == 256);
        createTable.Columns.ShouldContain(c => c.Name == "RevokedAt" && c.ClrType == typeof(DateTime) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "CreationTime" && c.ClrType == typeof(DateTime) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "CreatorId" && c.ClrType == typeof(Guid) && c.IsNullable);

        createTable.PrimaryKey.ShouldNotBeNull();
        createTable.PrimaryKey.Name.ShouldBe("PK_AppDeviceRevocations");
        createTable.PrimaryKey.Columns.ShouldBe(new[] { "Id" });

        var index = operations.OfType<CreateIndexOperation>().FirstOrDefault(i => i.Name == "IX_AppDeviceRevocations_WorkspaceId_UserId_DeviceId");
        index.ShouldNotBeNull();
        index.Table.ShouldBe("AppDeviceRevocations");
        index.IsUnique.ShouldBeTrue();
        index.Columns.ShouldBe(new[] { "WorkspaceId", "UserId", "DeviceId" });
    }

    [Fact]
    public void Down_Operations_Reversibly_Drop_AppDeviceRevocations_Table()
    {
        var downOps = GetDownOperations();

        downOps.OfType<DropTableOperation>()
            .Any(t => t.Name == "AppDeviceRevocations")
            .ShouldBeTrue();
    }

    [Fact]
    public void Current_Runtime_Model_Contains_DeviceRevocation_Entity()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var entity = context.Model.FindEntityType(typeof(DeviceRevocation));
        entity.ShouldNotBeNull();
        entity.GetTableName().ShouldBe("AppDeviceRevocations");

        entity.FindProperty(nameof(DeviceRevocation.WorkspaceId)).ShouldNotBeNull();
        entity.FindProperty(nameof(DeviceRevocation.UserId)).ShouldNotBeNull();
        entity.FindProperty(nameof(DeviceRevocation.DeviceId)).ShouldNotBeNull();
        entity.FindProperty(nameof(DeviceRevocation.Reason)).ShouldNotBeNull();
        entity.FindProperty(nameof(DeviceRevocation.RevokedAt)).ShouldNotBeNull();

        var index = entity.GetIndexes().FirstOrDefault(i => i.IsUnique && i.Properties.Select(p => p.Name).SequenceEqual(new[] { "WorkspaceId", "UserId", "DeviceId" }));
        index.ShouldNotBeNull();
    }

    [Fact]
    public void Direct_Tracked_ModelSnapshot_Contains_DeviceRevocation()
    {
        var snapshot = new TrackerDbContextModelSnapshot();
        var model = snapshot.Model;
        model.ShouldNotBeNull();

        var entity = model.FindEntityType("Dosi.Tracker.Devices.DeviceRevocation")
            ?? model.FindEntityType(typeof(DeviceRevocation));
        entity.ShouldNotBeNull();
        entity.GetTableName().ShouldBe("AppDeviceRevocations");
    }

    [Fact]
    public void Generated_PostgreSql_Up_Ddl_Contains_CreateTable_And_UniqueIndex()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
        var commands = sqlGenerator.Generate(GetUpOperations());
        var ddl = string.Join("\n", commands.Select(c => c.CommandText));

        ddl.ShouldContain("CREATE TABLE \"AppDeviceRevocations\"");
        ddl.ShouldContain("CREATE UNIQUE INDEX \"IX_AppDeviceRevocations_WorkspaceId_UserId_DeviceId\" ON \"AppDeviceRevocations\"");
    }

    [Fact]
    public async Task Tracked_Baseline_To_Current_Migrator_Upgrade_Preserves_Existing_Data_And_Enables_DeviceRevocations()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();

        var sqliteOptions = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseSqlite(connection)
            .Options;

        // 1. Migrate to the baseline (prior migration: 20261020200944_AddWorkspaceArchivalFields)
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync("20261020200944_AddWorkspaceArchivalFields");
        }

        // Verify baseline state
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM \"__EFMigrationsHistory\" WHERE \"MigrationId\" = '20261020200944_AddWorkspaceArchivalFields';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);

            // AppDeviceRevocations should NOT exist yet
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'AppDeviceRevocations';";
            var tableCount = (long)(await cmd.ExecuteScalarAsync())!;
            tableCount.ShouldBe(0L);
        }

        // 2. Seed baseline rows (Workspace and WorkspaceMember)
        var wsId = Guid.NewGuid();
        var memberId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var cTime = DateTime.UtcNow.ToString("o");

        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                INSERT INTO ""AppWorkspaces"" (""Id"", ""Name"", ""NormalizedName"", ""IsArchived"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@wsId, 'Device WS', 'DEVICE WS', 0, '{}', 'stamp1', @cTime);

                INSERT INTO ""AppWorkspaceMembers"" (""Id"", ""WorkspaceId"", ""UserId"", ""Role"", ""Status"", ""JoinedAt"", ""CreationTime"")
                VALUES (@memberId, @wsId, @userId, 'Owner', 'Active', @cTime, @cTime);
            ";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@userId", userId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@cTime", cTime));
            await cmd.ExecuteNonQueryAsync();
        }

        // 3. Migrate to current migration (20261021060000_AddDeviceRevocations)
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync("20261021060000_AddDeviceRevocations");
        }

        // 4. Verify table and unique index exist
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'AppDeviceRevocations';";
            var tableCount = (long)(await cmd.ExecuteScalarAsync())!;
            tableCount.ShouldBe(1L);

            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'IX_AppDeviceRevocations_WorkspaceId_UserId_DeviceId';";
            var indexCount = (long)(await cmd.ExecuteScalarAsync())!;
            indexCount.ShouldBe(1L);
        }

        // 5. Verify existing baseline data survived intact
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Name\", \"IsArchived\" FROM \"AppWorkspaces\" WHERE \"Id\" = @wsId;";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Device WS");
            reader.GetInt64(1).ShouldBe(0L);
        }

        // 6. Verify production TrackerDbContext can persist and query DeviceRevocation
        var deviceId = Guid.NewGuid();
        var revocationId = Guid.NewGuid();
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var revocation = new DeviceRevocation(revocationId, wsId, userId, deviceId, DateTime.UtcNow, "Test lost device");
            await context.DeviceRevocations.AddAsync(revocation);
            await context.SaveChangesAsync();
        }

        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var loaded = await context.DeviceRevocations.IgnoreQueryFilters().FirstOrDefaultAsync(r => r.WorkspaceId == wsId && r.UserId == userId && r.DeviceId == deviceId);
            loaded.ShouldNotBeNull();
            loaded.Reason.ShouldBe("Test lost device");
            loaded.DeviceId.ShouldBe(deviceId);
        }
    }
}
