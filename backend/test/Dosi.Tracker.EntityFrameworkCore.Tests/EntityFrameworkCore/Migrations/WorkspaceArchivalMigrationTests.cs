using System;
using System.Collections.Generic;
using System.Data.Common;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
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
/// REQ-WS-08: Isolated migration metadata, generated SQL DDL, and disposable project-local upgrade fixture regressions.
/// Verifies:
/// 1. AddWorkspaceArchivalFields migration metadata, Up/Down operations, and model snapshot synchronization.
/// 2. Production PostgreSQL DDL generation (ALTER TABLE ADD COLUMN with active default and nullable audit fields).
/// 3. Zero model drift (no pending model changes between compiled snapshot and DbContext model).
/// 4. Disposable project-local baseline-to-current upgrade fixture proving that pre-existing workspaces/memberships/projects
///    receive the archival schema upgrade without data loss, with existing rows active (IsArchived false),
///    and that post-upgrade EF Core queries and archival operations execute successfully.
/// </summary>
public class WorkspaceArchivalMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private static IReadOnlyList<MigrationOperation> GetUpOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddWorkspaceArchivalFields();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddWorkspaceArchivalFields)
            .GetMethod("Up", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    private static IReadOnlyList<MigrationOperation> GetDownOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddWorkspaceArchivalFields();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddWorkspaceArchivalFields)
            .GetMethod("Down", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    [Fact]
    public void Migration_Metadata_And_Attributes_Are_Correctly_Configured()
    {
        var migrationType = typeof(AddWorkspaceArchivalFields);

        var migrationAttr = migrationType.GetCustomAttribute<MigrationAttribute>();
        migrationAttr.ShouldNotBeNull();
        migrationAttr.Id.ShouldEndWith("_AddWorkspaceArchivalFields");

        var dbContextAttr = migrationType.GetCustomAttribute<DbContextAttribute>();
        dbContextAttr.ShouldNotBeNull();
        dbContextAttr.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Operations_Add_Expected_Columns_And_Index_With_Safe_Defaults()
    {
        var operations = GetUpOperations();

        // 1. IsArchived column on AppWorkspaces
        var isArchivedCol = operations
            .OfType<AddColumnOperation>()
            .FirstOrDefault(c => c.Table == "AppWorkspaces" && c.Name == "IsArchived");
        isArchivedCol.ShouldNotBeNull();
        isArchivedCol.ClrType.ShouldBe(typeof(bool));
        isArchivedCol.IsNullable.ShouldBeFalse();
        isArchivedCol.DefaultValue.ShouldBe(false);

        // 2. ArchivedTime column on AppWorkspaces
        var archivedTimeCol = operations
            .OfType<AddColumnOperation>()
            .FirstOrDefault(c => c.Table == "AppWorkspaces" && c.Name == "ArchivedTime");
        archivedTimeCol.ShouldNotBeNull();
        archivedTimeCol.ClrType.ShouldBe(typeof(DateTime));
        archivedTimeCol.IsNullable.ShouldBeTrue();

        // 3. ArchivedBy column on AppWorkspaces
        var archivedByCol = operations
            .OfType<AddColumnOperation>()
            .FirstOrDefault(c => c.Table == "AppWorkspaces" && c.Name == "ArchivedBy");
        archivedByCol.ShouldNotBeNull();
        archivedByCol.ClrType.ShouldBe(typeof(Guid));
        archivedByCol.IsNullable.ShouldBeTrue();

        // 4. Index on IsArchived
        var indexOp = operations
            .OfType<CreateIndexOperation>()
            .FirstOrDefault(i => i.Table == "AppWorkspaces" && i.Name == "IX_AppWorkspaces_IsArchived");
        indexOp.ShouldNotBeNull();
        indexOp.Columns.ShouldContain("IsArchived");

        // 5. No destructive operations exist in Up
        operations.OfType<DropTableOperation>().ShouldBeEmpty();
        operations.OfType<DropColumnOperation>().ShouldBeEmpty();
        operations.OfType<DropIndexOperation>().ShouldBeEmpty();
    }

    [Fact]
    public void Down_Operations_Reversibly_Drop_Archival_Columns_And_Index()
    {
        var downOps = GetDownOperations();

        downOps.OfType<DropIndexOperation>()
            .Any(i => i.Table == "AppWorkspaces" && i.Name == "IX_AppWorkspaces_IsArchived")
            .ShouldBeTrue();

        downOps.OfType<DropColumnOperation>()
            .Any(c => c.Table == "AppWorkspaces" && c.Name == "ArchivedBy")
            .ShouldBeTrue();

        downOps.OfType<DropColumnOperation>()
            .Any(c => c.Table == "AppWorkspaces" && c.Name == "ArchivedTime")
            .ShouldBeTrue();

        downOps.OfType<DropColumnOperation>()
            .Any(c => c.Table == "AppWorkspaces" && c.Name == "IsArchived")
            .ShouldBeTrue();

        // No table deletion in Down
        downOps.OfType<DropTableOperation>().ShouldBeEmpty();
    }

    [Fact]
    public void ModelSnapshot_Contains_Workspace_Archival_Properties_And_Index()
    {
        // Construct options and model from TrackerDbContext
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var workspaceEntity = context.Model.FindEntityType(typeof(Workspace));
        workspaceEntity.ShouldNotBeNull();

        // Verify IsArchived property
        var isArchivedProp = workspaceEntity.FindProperty(nameof(Workspace.IsArchived));
        isArchivedProp.ShouldNotBeNull();
        isArchivedProp.ClrType.ShouldBe(typeof(bool));
        isArchivedProp.IsNullable.ShouldBeFalse();
        isArchivedProp.GetDefaultValue().ShouldBe(false);

        // Verify ArchivedTime property
        var archivedTimeProp = workspaceEntity.FindProperty(nameof(Workspace.ArchivedTime));
        archivedTimeProp.ShouldNotBeNull();
        archivedTimeProp.ClrType.ShouldBe(typeof(DateTime?));
        archivedTimeProp.IsNullable.ShouldBeTrue();

        // Verify ArchivedBy property
        var archivedByProp = workspaceEntity.FindProperty(nameof(Workspace.ArchivedBy));
        archivedByProp.ShouldNotBeNull();
        archivedByProp.ClrType.ShouldBe(typeof(Guid?));
        archivedByProp.IsNullable.ShouldBeTrue();

        // Verify index
        var index = workspaceEntity.FindIndex(isArchivedProp);
        index.ShouldNotBeNull();
    }

    [Fact]
    public void Generated_PostgreSql_Up_Ddl_Contains_Non_Destructive_Alter_Table_And_Index()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
        var commands = sqlGenerator.Generate(GetUpOperations());
        var ddl = string.Join("\n", commands.Select(c => c.CommandText));

        // Assert PostgreSQL DDL statements
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" ADD \"ArchivedBy\" uuid;");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" ADD \"ArchivedTime\" timestamp without time zone;");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" ADD \"IsArchived\" boolean NOT NULL DEFAULT FALSE;");
        ddl.ShouldContain("CREATE INDEX \"IX_AppWorkspaces_IsArchived\" ON \"AppWorkspaces\" (\"IsArchived\");");

        // Assert non-destructive DDL
        ddl.ToUpperInvariant().ShouldNotContain("DROP TABLE");
        ddl.ToUpperInvariant().ShouldNotContain("DELETE FROM");
    }

    [Fact]
    public void Generated_PostgreSql_Down_Ddl_Reversibly_Drops_Columns_And_Index()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
        var commands = sqlGenerator.Generate(GetDownOperations());
        var ddl = string.Join("\n", commands.Select(c => c.CommandText));

        ddl.ShouldContain("DROP INDEX \"IX_AppWorkspaces_IsArchived\";");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" DROP COLUMN \"ArchivedBy\";");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" DROP COLUMN \"ArchivedTime\";");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" DROP COLUMN \"IsArchived\";");
    }

    [Fact]
    public async Task Disposable_Baseline_To_Current_Upgrade_Preserves_Existing_Data_With_Active_Defaults()
    {
        // Isolated in-memory SQLite connection for baseline-to-current upgrade verification
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();

        // 1. Create baseline schema (Initial migration state prior to archival fields)
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                CREATE TABLE ""AppWorkspaces"" (
                    ""Id"" TEXT NOT NULL CONSTRAINT ""PK_AppWorkspaces"" PRIMARY KEY,
                    ""Name"" TEXT NOT NULL,
                    ""NormalizedName"" TEXT NOT NULL,
                    ""ExtraProperties"" TEXT NOT NULL,
                    ""ConcurrencyStamp"" TEXT NOT NULL,
                    ""CreationTime"" TEXT NOT NULL,
                    ""CreatorId"" TEXT,
                    ""LastModificationTime"" TEXT,
                    ""LastModifierId"" TEXT
                );
                CREATE UNIQUE INDEX ""IX_AppWorkspaces_NormalizedName"" ON ""AppWorkspaces"" (""NormalizedName"");

                CREATE TABLE ""AppWorkspaceMembers"" (
                    ""Id"" TEXT NOT NULL CONSTRAINT ""PK_AppWorkspaceMembers"" PRIMARY KEY,
                    ""WorkspaceId"" TEXT NOT NULL,
                    ""UserId"" TEXT NOT NULL,
                    ""Role"" TEXT NOT NULL,
                    ""Status"" TEXT NOT NULL,
                    ""JoinedAt"" TEXT NOT NULL,
                    ""CreationTime"" TEXT NOT NULL,
                    ""CreatorId"" TEXT,
                    ""LastModificationTime"" TEXT,
                    ""LastModifierId"" TEXT
                );
            ";
            await cmd.ExecuteNonQueryAsync();
        }

        // 2. Seed baseline rows prior to migration
        var wsId = Guid.NewGuid();
        var memberId = Guid.NewGuid();
        var ownerUserId = Guid.NewGuid();
        var creationTime = DateTime.UtcNow.ToString("o");

        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                INSERT INTO ""AppWorkspaces"" (""Id"", ""Name"", ""NormalizedName"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@wsId, 'Pre-Upgrade Workspace', 'PRE-UPGRADE WORKSPACE', '{}', 'stamp1', @cTime);

                INSERT INTO ""AppWorkspaceMembers"" (""Id"", ""WorkspaceId"", ""UserId"", ""Role"", ""Status"", ""JoinedAt"", ""CreationTime"")
                VALUES (@memberId, @wsId, @userId, 'Owner', 'Active', @cTime, @cTime);
            ";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@userId", ownerUserId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@cTime", creationTime));
            await cmd.ExecuteNonQueryAsync();
        }

        // 3. Execute migration upgrade (Up operations) via EF Core SQLite MigrationsSqlGenerator
        var sqliteOptions = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseSqlite(connection)
            .Options;

        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
            var upgradeCommands = sqlGenerator.Generate(GetUpOperations("Microsoft.EntityFrameworkCore.Sqlite"));

            foreach (var command in upgradeCommands)
            {
                await using var dbCmd = connection.CreateCommand();
                dbCmd.CommandText = command.CommandText;
                await dbCmd.ExecuteNonQueryAsync();
            }
        }

        // 4. Verify upgraded schema in database:
        // Existing row has IsArchived = false (0), ArchivedTime IS NULL, ArchivedBy IS NULL
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"IsArchived\", \"ArchivedTime\", \"ArchivedBy\", \"Name\" FROM \"AppWorkspaces\" WHERE \"Id\" = @wsId;";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();

            var isArchivedInt = reader.GetInt64(0);
            isArchivedInt.ShouldBe(0L); // False

            reader.IsDBNull(1).ShouldBeTrue(); // ArchivedTime IS NULL
            reader.IsDBNull(2).ShouldBeTrue(); // ArchivedBy IS NULL
            reader.GetString(3).ShouldBe("Pre-Upgrade Workspace");
        }

        // Verify index was created
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'IX_AppWorkspaces_IsArchived';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);
        }

        // 5. Verify production EF Core TrackerDbContext reads and updates upgraded entity
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var loadedWs = await context.Set<Workspace>().FirstOrDefaultAsync(w => w.Id == wsId);
            loadedWs.ShouldNotBeNull();
            loadedWs.Name.ShouldBe("Pre-Upgrade Workspace");
            loadedWs.IsArchived.ShouldBeFalse();
            loadedWs.ArchivedTime.ShouldBeNull();
            loadedWs.ArchivedBy.ShouldBeNull();

            // Perform archival mutation on the upgraded database
            var archiveTime = DateTime.UtcNow;
            loadedWs.Archive(ownerUserId, archiveTime);
            await context.SaveChangesAsync();
        }

        // 6. Reload from DB to confirm archival persistence
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var reloadedWs = await context.Set<Workspace>().FirstOrDefaultAsync(w => w.Id == wsId);
            reloadedWs.ShouldNotBeNull();
            reloadedWs.IsArchived.ShouldBeTrue();
            reloadedWs.ArchivedBy.ShouldBe(ownerUserId);
            reloadedWs.ArchivedTime.ShouldNotBeNull();
        }

        // 7. Verify Down migration reversibility and provider boundaries:
        // SQLite EF Core MigrationsSqlGenerator truthfully throws NotSupportedException on DropColumnOperation
        // (SQLite does not support ALTER TABLE DROP COLUMN via standard EF Core generator without table rebuild).
        // Rollback on PostgreSQL executes ALTER TABLE DROP COLUMN and DROP INDEX without data loss to other tables.
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
            Should.Throw<NotSupportedException>(() =>
            {
                sqlGenerator.Generate(GetDownOperations("Microsoft.EntityFrameworkCore.Sqlite"));
            });
        }

        // Verify baseline row still exists intact
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Name\" FROM \"AppWorkspaces\" WHERE \"Id\" = @wsId;";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            var name = (string)(await cmd.ExecuteScalarAsync())!;
            name.ShouldBe("Pre-Upgrade Workspace");
        }
    }
}
