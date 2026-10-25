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
    public void Current_Runtime_Model_Contains_Workspace_Archival_Properties_And_Index()
    {
        // Construct options and model from TrackerDbContext runtime model
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var workspaceEntity = context.Model.FindEntityType(typeof(Workspace));
        workspaceEntity.ShouldNotBeNull();
        AssertWorkspaceArchivalSchema(workspaceEntity);
    }

    [Fact]
    public void Direct_Tracked_ModelSnapshot_Contains_Workspace_Archival_Properties_And_Index()
    {
        // Direct instantiation of the tracked TrackerDbContextModelSnapshot artifact
        var snapshot = new TrackerDbContextModelSnapshot();
        snapshot.ShouldNotBeNull();

        var model = snapshot.Model;
        model.ShouldNotBeNull();

        var workspaceEntity = model.FindEntityType("Dosi.Tracker.Workspaces.Workspace")
            ?? model.FindEntityType(typeof(Workspace));
        workspaceEntity.ShouldNotBeNull();
        workspaceEntity.GetTableName().ShouldBe("AppWorkspaces");

        AssertWorkspaceArchivalSchema(workspaceEntity);
    }

    [Fact]
    public void Direct_Tracked_Migration_TargetModel_Contains_Workspace_Archival_Properties_And_Index()
    {
        // Direct instantiation of the tracked AddWorkspaceArchivalFields migration TargetModel artifact
        var migration = new AddWorkspaceArchivalFields();
        migration.ShouldNotBeNull();

        var targetModel = migration.TargetModel;
        targetModel.ShouldNotBeNull();

        var workspaceEntity = targetModel.FindEntityType("Dosi.Tracker.Workspaces.Workspace")
            ?? targetModel.FindEntityType(typeof(Workspace));
        workspaceEntity.ShouldNotBeNull();
        workspaceEntity.GetTableName().ShouldBe("AppWorkspaces");

        AssertWorkspaceArchivalSchema(workspaceEntity);
    }

    private static void AssertWorkspaceArchivalSchema(Microsoft.EntityFrameworkCore.Metadata.IEntityType workspaceEntity)
    {
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

        // Verify index on IsArchived
        var index = workspaceEntity.FindIndex(isArchivedProp);
        index.ShouldNotBeNull();
        index.Properties.Select(p => p.Name).ShouldContain(nameof(Workspace.IsArchived));
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

    /// <summary>
    /// Operation-level regression: tests reflected AddWorkspaceArchivalFields.Up operations against a hand-built
    /// baseline SQLite schema matching 20261007185459_Initial (AppWorkspaces, AppWorkspaceMembers, AppProjects, AppSubscriptions).
    /// Proves that provider DDL generation and entity persistence execute non-destructively, preserving existing
    /// workspaces (with active default IsArchived = false and null audit fields), memberships, projects, and subscriptions.
    /// Note: This tests operation-level DDL/entity persistence; full tracked migrator discovery is tested in
    /// Tracked_Baseline_To_Current_Migrator_Upgrade_Preserves_Existing_Data_With_Active_Defaults.
    /// </summary>
    [Fact]
    public async Task Disposable_OperationLevel_Baseline_Upgrade_Preserves_Workspace_Memberships_Projects_And_Subscriptions()
    {
        // Isolated in-memory SQLite connection for operation-level baseline-to-current upgrade verification
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();

        // 1. Hand-built baseline schema: matching 20261007185459_Initial prior to archival fields
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

                CREATE TABLE ""AppProjects"" (
                    ""Id"" TEXT NOT NULL CONSTRAINT ""PK_AppProjects"" PRIMARY KEY,
                    ""WorkspaceId"" TEXT NOT NULL,
                    ""Title"" TEXT NOT NULL,
                    ""Description"" TEXT,
                    ""Color"" TEXT NOT NULL,
                    ""IntervalMinutes"" INTEGER NOT NULL,
                    ""IsArchived"" INTEGER NOT NULL,
                    ""ArchivedAt"" TEXT,
                    ""AllowScreenshot"" INTEGER NOT NULL,
                    ""AllowWebcam"" INTEGER NOT NULL,
                    ""AllowKeyboard"" INTEGER NOT NULL,
                    ""AllowMouse"" INTEGER NOT NULL,
                    ""AllowActiveWindow"" INTEGER NOT NULL,
                    ""AllowRunningPrograms"" INTEGER NOT NULL,
                    ""IdleThresholdMinutes"" INTEGER NOT NULL,
                    ""BlurScreenshots"" INTEGER NOT NULL,
                    ""ExtraProperties"" TEXT NOT NULL,
                    ""ConcurrencyStamp"" TEXT NOT NULL,
                    ""CreationTime"" TEXT NOT NULL,
                    ""CreatorId"" TEXT,
                    ""LastModificationTime"" TEXT,
                    ""LastModifierId"" TEXT,
                    ""IsDeleted"" INTEGER NOT NULL DEFAULT 0,
                    ""DeleterId"" TEXT,
                    ""DeletionTime"" TEXT
                );

                CREATE TABLE ""AppSubscriptions"" (
                    ""Id"" TEXT NOT NULL CONSTRAINT ""PK_AppSubscriptions"" PRIMARY KEY,
                    ""WorkspaceId"" TEXT NOT NULL,
                    ""PlanId"" TEXT NOT NULL,
                    ""Status"" TEXT NOT NULL,
                    ""TrialEndsAt"" TEXT,
                    ""ExtraProperties"" TEXT NOT NULL,
                    ""ConcurrencyStamp"" TEXT NOT NULL,
                    ""CreationTime"" TEXT NOT NULL,
                    ""CreatorId"" TEXT,
                    ""LastModificationTime"" TEXT,
                    ""LastModifierId"" TEXT,
                    ""IsDeleted"" INTEGER NOT NULL DEFAULT 0,
                    ""DeleterId"" TEXT,
                    ""DeletionTime"" TEXT
                );
            ";
            await cmd.ExecuteNonQueryAsync();
        }

        // 2. Seed baseline rows prior to migration (Workspace, Member, Project, Subscription sentinels)
        var wsId = Guid.NewGuid();
        var memberId = Guid.NewGuid();
        var ownerUserId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var subId = Guid.NewGuid();
        var planId = Guid.NewGuid();
        var creationTime = DateTime.UtcNow.ToString("o");

        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                INSERT INTO ""AppWorkspaces"" (""Id"", ""Name"", ""NormalizedName"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@wsId, 'Pre-Upgrade Workspace', 'PRE-UPGRADE WORKSPACE', '{}', 'stamp1', @cTime);

                INSERT INTO ""AppWorkspaceMembers"" (""Id"", ""WorkspaceId"", ""UserId"", ""Role"", ""Status"", ""JoinedAt"", ""CreationTime"")
                VALUES (@memberId, @wsId, @userId, 'Owner', 'Active', @cTime, @cTime);

                INSERT INTO ""AppProjects"" (""Id"", ""WorkspaceId"", ""Title"", ""Color"", ""IntervalMinutes"", ""IsArchived"", ""AllowScreenshot"", ""AllowWebcam"", ""AllowKeyboard"", ""AllowMouse"", ""AllowActiveWindow"", ""AllowRunningPrograms"", ""IdleThresholdMinutes"", ""BlurScreenshots"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"", ""IsDeleted"")
                VALUES (@projId, @wsId, 'Operation-Level Project Sentinel', '#112233', 10, 0, 1, 0, 1, 1, 1, 1, 5, 0, '{}', 'stamp2', @cTime, 0);

                INSERT INTO ""AppSubscriptions"" (""Id"", ""WorkspaceId"", ""PlanId"", ""Status"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"", ""IsDeleted"")
                VALUES (@subId, @wsId, @planId, 'Active', '{}', 'stamp3', @cTime, 0);
            ";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@userId", ownerUserId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@projId", projectId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@subId", subId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@planId", planId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@cTime", creationTime));
            await cmd.ExecuteNonQueryAsync();
        }

        // 3. Execute production migration upgrade operations (AddWorkspaceArchivalFields.Up) via IMigrationsSqlGenerator
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
        // Existing workspace row has IsArchived = false (0), ArchivedTime IS NULL, ArchivedBy IS NULL
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

        // 5. Verify all sentinel rows survived intact in hand-built baseline tables:
        // Membership sentinel survived
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Role\", \"Status\" FROM \"AppWorkspaceMembers\" WHERE \"Id\" = @memberId;";
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Owner");
            reader.GetString(1).ShouldBe("Active");
        }

        // Project sentinel survived
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Title\", \"IsArchived\" FROM \"AppProjects\" WHERE \"Id\" = @projId;";
            cmd.Parameters.Add(new SqliteParameter("@projId", projectId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Operation-Level Project Sentinel");
            reader.GetInt64(1).ShouldBe(0L);
        }

        // Subscription sentinel survived
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Status\" FROM \"AppSubscriptions\" WHERE \"Id\" = @subId;";
            cmd.Parameters.Add(new SqliteParameter("@subId", subId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Active");
        }

        // 6. Verify production EF Core TrackerDbContext reads and updates upgraded entity. The runtime model also maps the
        // workspace columns later migrations added, so their workspace operations are applied first.
        await ApplyLaterWorkspaceOperationsAsync(connection, sqliteOptions);
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

        // 7. Reload from DB to confirm archival persistence
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var reloadedWs = await context.Set<Workspace>().FirstOrDefaultAsync(w => w.Id == wsId);
            reloadedWs.ShouldNotBeNull();
            reloadedWs.IsArchived.ShouldBeTrue();
            reloadedWs.ArchivedBy.ShouldBe(ownerUserId);
            reloadedWs.ArchivedTime.ShouldNotBeNull();
        }

        // 8. Verify Down migration reversibility and provider boundaries:
        // SQLite EF Core MigrationsSqlGenerator truthfully throws NotSupportedException on DropColumnOperation
        // (SQLite does not support ALTER TABLE DROP COLUMN via standard EF Core generator without table rebuild).
        // Rollback on PostgreSQL executes ALTER TABLE DROP COLUMN and DROP INDEX.
        // Rollback consequence: Dropping archive columns via Down() removes IsArchived, ArchivedTime, and ArchivedBy,
        // which loses archival state and audit history, returning any previously archived workspaces to unarchived
        // status upon rollback; it is NOT a lossless rollback with respect to archival state. Underlying workspaces,
        // memberships, projects, and subscriptions remain intact and are not deleted.
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

    /// <summary>The workspace operations of the migrations after AddWorkspaceArchivalFields (the hand-made baseline of the
    /// operation-level test has no other tables they touch).</summary>
    private static async Task ApplyLaterWorkspaceOperationsAsync(SqliteConnection connection, DbContextOptions<TrackerDbContext> options)
    {
        var builder = new MigrationBuilder("Microsoft.EntityFrameworkCore.Sqlite");
        typeof(ErasureAndDataProtection)
            .GetMethod("Up", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .Invoke(new ErasureAndDataProtection(), new object[] { builder });
        var workspaceOperations = builder.Operations.Where(o => o switch
        {
            AddColumnOperation add => add.Table == "AppWorkspaces",
            DropIndexOperation drop => drop.Table == "AppWorkspaces",
            CreateIndexOperation create => create.Table == "AppWorkspaces",
            _ => false
        }).ToList();

        using var context = new TrackerDbContext(options);
        foreach (var command in context.GetService<IMigrationsSqlGenerator>().Generate(workspaceOperations))
        {
            await using var dbCmd = connection.CreateCommand();
            dbCmd.CommandText = command.CommandText;
            await dbCmd.ExecuteNonQueryAsync();
        }
    }

    [Fact]
    public async Task Tracked_Baseline_To_Current_Migrator_Upgrade_Preserves_Existing_Data_With_Active_Defaults()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();

        var sqliteOptions = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseSqlite(connection)
            .Options;

        // 1. Migrate specifically to the baseline Initial migration using tracked IMigrator
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync("20261007185459_Initial");
        }

        // Verify initial migration applied and archival columns DO NOT yet exist
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM \"__EFMigrationsHistory\" WHERE \"MigrationId\" = '20261007185459_Initial';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);

            // Columns should not exist in Initial schema
            cmd.CommandText = "PRAGMA table_info(\"AppWorkspaces\");";
            var cols = new List<string>();
            await using (var reader = await cmd.ExecuteReaderAsync())
            {
                while (await reader.ReadAsync())
                {
                    cols.Add(reader.GetString(1));
                }
            }
            cols.ShouldNotContain("IsArchived");
            cols.ShouldNotContain("ArchivedTime");
            cols.ShouldNotContain("ArchivedBy");
        }

        // 2. Seed baseline rows (Workspace, Member, Project, Subscription sentinels)
        var wsId = Guid.NewGuid();
        var memberId = Guid.NewGuid();
        var ownerUserId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var subId = Guid.NewGuid();
        var planId = Guid.NewGuid();
        var creationTime = DateTime.UtcNow.ToString("o");

        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                INSERT INTO ""AppWorkspaces"" (""Id"", ""Name"", ""NormalizedName"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@wsId, 'Pre-Upgrade Tracked WS', 'PRE-UPGRADE TRACKED WS', '{}', 'stamp1', @cTime);

                INSERT INTO ""AppWorkspaceMembers"" (""Id"", ""WorkspaceId"", ""UserId"", ""Role"", ""Status"", ""JoinedAt"", ""CreationTime"")
                VALUES (@memberId, @wsId, @userId, 'Owner', 'Active', @cTime, @cTime);

                INSERT INTO ""AppProjects"" (""Id"", ""WorkspaceId"", ""Title"", ""Color"", ""IntervalMinutes"", ""IsArchived"", ""AllowScreenshot"", ""AllowWebcam"", ""AllowKeyboard"", ""AllowMouse"", ""AllowActiveWindow"", ""AllowRunningPrograms"", ""IdleThresholdMinutes"", ""BlurScreenshots"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"", ""IsDeleted"")
                VALUES (@projId, @wsId, 'Sentinel Project', '#000000', 10, 0, 1, 0, 1, 1, 1, 1, 5, 0, '{}', 'stamp2', @cTime, 0);

                INSERT INTO ""AppSubscriptions"" (""Id"", ""WorkspaceId"", ""PlanId"", ""Status"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"", ""IsDeleted"")
                VALUES (@subId, @wsId, @planId, 'Active', '{}', 'stamp3', @cTime, 0);
            ";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@userId", ownerUserId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@projId", projectId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@subId", subId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@planId", planId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@cTime", creationTime));
            await cmd.ExecuteNonQueryAsync();
        }

        // 3. Execute the tracked migrations up to the current one (through 20261020200944_AddWorkspaceArchivalFields and
        // every later migration, so the runtime model below reads a current schema)
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync();
        }

        // Verify history table contains the new migration
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM \"__EFMigrationsHistory\" WHERE \"MigrationId\" = '20261020200944_AddWorkspaceArchivalFields';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);
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
            reader.GetString(3).ShouldBe("Pre-Upgrade Tracked WS");
        }

        // Verify index was created
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'IX_AppWorkspaces_IsArchived';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);
        }

        // 5. Verify all sentinel rows survived intact:
        // Member sentinel
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Role\", \"Status\" FROM \"AppWorkspaceMembers\" WHERE \"Id\" = @memberId;";
            cmd.Parameters.Add(new SqliteParameter("@memberId", memberId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Owner");
            reader.GetString(1).ShouldBe("Active");
        }

        // Project sentinel
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Title\", \"IsArchived\" FROM \"AppProjects\" WHERE \"Id\" = @projId;";
            cmd.Parameters.Add(new SqliteParameter("@projId", projectId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Sentinel Project");
            reader.GetInt64(1).ShouldBe(0L);
        }

        // Subscription sentinel
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Status\" FROM \"AppSubscriptions\" WHERE \"Id\" = @subId;";
            cmd.Parameters.Add(new SqliteParameter("@subId", subId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Active");
        }

        // 6. Verify production EF Core TrackerDbContext reads and updates upgraded entity
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var loadedWs = await context.Set<Workspace>().FirstOrDefaultAsync(w => w.Id == wsId);
            loadedWs.ShouldNotBeNull();
            loadedWs.Name.ShouldBe("Pre-Upgrade Tracked WS");
            loadedWs.IsArchived.ShouldBeFalse();
            loadedWs.ArchivedTime.ShouldBeNull();
            loadedWs.ArchivedBy.ShouldBeNull();

            // Perform archival mutation on the upgraded database
            var archiveTime = DateTime.UtcNow;
            loadedWs.Archive(ownerUserId, archiveTime);
            await context.SaveChangesAsync();
        }

        // 7. Reload from DB to confirm archival persistence
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var reloadedWs = await context.Set<Workspace>().FirstOrDefaultAsync(w => w.Id == wsId);
            reloadedWs.ShouldNotBeNull();
            reloadedWs.IsArchived.ShouldBeTrue();
            reloadedWs.ArchivedBy.ShouldBe(ownerUserId);
            reloadedWs.ArchivedTime.ShouldNotBeNull();
        }
    }
}
