using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.Migrations;
using Dosi.Tracker.Projects;
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
/// Project task schema migration tests:
/// 1. AddProjectTasks migration metadata, Up/Down operations, and model snapshot synchronization.
/// 2. Production PostgreSQL DDL generation (CREATE TABLE "AppProjectTasks" with indexes).
/// 3. Tracked baseline-to-current upgrade regression preserving existing workspaces, members, and data,
///    verifying successful creation and querying of ProjectTask entities post-upgrade.
/// </summary>
public class ProjectTaskMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private static IReadOnlyList<MigrationOperation> GetUpOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddProjectTasks();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddProjectTasks)
            .GetMethod("Up", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    private static IReadOnlyList<MigrationOperation> GetDownOperations(string activeProvider = "Npgsql.EntityFrameworkCore.PostgreSQL")
    {
        var migration = new AddProjectTasks();
        var builder = new MigrationBuilder(activeProvider);
        typeof(AddProjectTasks)
            .GetMethod("Down", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(migration, new object[] { builder });
        return builder.Operations;
    }

    [Fact]
    public void Migration_Metadata_And_Attributes_Are_Correctly_Configured()
    {
        var migrationType = typeof(AddProjectTasks);

        var migrationAttr = migrationType.GetCustomAttribute<MigrationAttribute>();
        migrationAttr.ShouldNotBeNull();
        migrationAttr.Id.ShouldEndWith("_AddProjectTasks");

        var dbContextAttr = migrationType.GetCustomAttribute<DbContextAttribute>();
        dbContextAttr.ShouldNotBeNull();
        dbContextAttr.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Operations_Create_AppProjectTasks_Table_And_Indexes()
    {
        var operations = GetUpOperations();

        var createTable = operations.OfType<CreateTableOperation>().FirstOrDefault(t => t.Name == "AppProjectTasks");
        createTable.ShouldNotBeNull();

        createTable.Columns.ShouldContain(c => c.Name == "Id" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "WorkspaceId" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "ProjectId" && c.ClrType == typeof(Guid) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "Title" && c.ClrType == typeof(string) && !c.IsNullable && c.MaxLength == 256);
        createTable.Columns.ShouldContain(c => c.Name == "Description" && c.ClrType == typeof(string) && c.IsNullable && c.MaxLength == 2048);
        createTable.Columns.ShouldContain(c => c.Name == "IsCompleted" && c.ClrType == typeof(bool) && !c.IsNullable);
        createTable.Columns.ShouldContain(c => c.Name == "CompletedAt" && c.ClrType == typeof(DateTime) && c.IsNullable);

        createTable.PrimaryKey.ShouldNotBeNull();
        createTable.PrimaryKey.Name.ShouldBe("PK_AppProjectTasks");
        createTable.PrimaryKey.Columns.ShouldBe(new[] { "Id" });

        var index1 = operations.OfType<CreateIndexOperation>().FirstOrDefault(i => i.Name == "IX_AppProjectTasks_WorkspaceId_ProjectId");
        index1.ShouldNotBeNull();
        index1.Table.ShouldBe("AppProjectTasks");
        index1.Columns.ShouldBe(new[] { "WorkspaceId", "ProjectId" });

        var index2 = operations.OfType<CreateIndexOperation>().FirstOrDefault(i => i.Name == "IX_AppProjectTasks_WorkspaceId_ProjectId_IsCompleted");
        index2.ShouldNotBeNull();
        index2.Table.ShouldBe("AppProjectTasks");
        index2.Columns.ShouldBe(new[] { "WorkspaceId", "ProjectId", "IsCompleted" });
    }

    [Fact]
    public void Down_Operations_Reversibly_Drop_AppProjectTasks_Table()
    {
        var downOps = GetDownOperations();

        downOps.OfType<DropTableOperation>()
            .Any(t => t.Name == "AppProjectTasks")
            .ShouldBeTrue();
    }

    [Fact]
    public void Current_Runtime_Model_Contains_ProjectTask_Entity()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var entity = context.Model.FindEntityType(typeof(ProjectTask));
        entity.ShouldNotBeNull();
        entity.GetTableName().ShouldBe("AppProjectTasks");

        entity.FindProperty(nameof(ProjectTask.WorkspaceId)).ShouldNotBeNull();
        entity.FindProperty(nameof(ProjectTask.ProjectId)).ShouldNotBeNull();
        entity.FindProperty(nameof(ProjectTask.Title)).ShouldNotBeNull();
        entity.FindProperty(nameof(ProjectTask.Description)).ShouldNotBeNull();
        entity.FindProperty(nameof(ProjectTask.IsCompleted)).ShouldNotBeNull();
        entity.FindProperty(nameof(ProjectTask.CompletedAt)).ShouldNotBeNull();

        var index = entity.GetIndexes().FirstOrDefault(i => i.Properties.Select(p => p.Name).SequenceEqual(new[] { "WorkspaceId", "ProjectId" }));
        index.ShouldNotBeNull();
    }

    [Fact]
    public void Direct_Tracked_ModelSnapshot_Contains_ProjectTask()
    {
        var snapshot = new TrackerDbContextModelSnapshot();
        var model = snapshot.Model;
        model.ShouldNotBeNull();

        var entity = model.FindEntityType("Dosi.Tracker.Projects.ProjectTask")
            ?? model.FindEntityType(typeof(ProjectTask));
        entity.ShouldNotBeNull();
        entity.GetTableName().ShouldBe("AppProjectTasks");
    }

    [Fact]
    public void Generated_PostgreSql_Up_Ddl_Contains_CreateTable_And_Indexes()
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseNpgsql("Host=localhost;Database=dummy;")
            .Options;

        using var context = new TrackerDbContext(options);
        var sqlGenerator = context.GetService<IMigrationsSqlGenerator>();
        var commands = sqlGenerator.Generate(GetUpOperations());
        var ddl = string.Join("\n", commands.Select(c => c.CommandText));

        ddl.ShouldContain("CREATE TABLE \"AppProjectTasks\"");
        ddl.ShouldContain("CREATE INDEX \"IX_AppProjectTasks_WorkspaceId_ProjectId\" ON \"AppProjectTasks\"");
        ddl.ShouldContain("CREATE INDEX \"IX_AppProjectTasks_WorkspaceId_ProjectId_IsCompleted\" ON \"AppProjectTasks\"");
    }

    [Fact]
    public async Task Tracked_Baseline_To_Current_Migrator_Upgrade_Preserves_Existing_Data_And_Enables_ProjectTasks()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();

        var sqliteOptions = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseSqlite(connection)
            .Options;

        // 1. Migrate to the baseline (prior migration: 20261021060000_AddDeviceRevocations)
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync("20261021060000_AddDeviceRevocations");
        }

        // Verify baseline state
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM \"__EFMigrationsHistory\" WHERE \"MigrationId\" = '20261021060000_AddDeviceRevocations';";
            var count = (long)(await cmd.ExecuteScalarAsync())!;
            count.ShouldBe(1L);

            // AppProjectTasks should NOT exist yet
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'AppProjectTasks';";
            var tableCount = (long)(await cmd.ExecuteScalarAsync())!;
            tableCount.ShouldBe(0L);
        }

        // 2. Seed baseline rows (Workspace and Project)
        var wsId = Guid.NewGuid();
        var projId = Guid.NewGuid();
        var cTime = DateTime.UtcNow.ToString("o");

        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = @"
                INSERT INTO ""AppWorkspaces"" (""Id"", ""Name"", ""NormalizedName"", ""IsArchived"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@wsId, 'Task WS', 'TASK WS', 0, '{}', 'stamp1', @cTime);

                INSERT INTO ""AppProjects"" (""Id"", ""WorkspaceId"", ""Title"", ""Color"", ""IntervalMinutes"", ""IsArchived"", ""AllowScreenshot"", ""AllowWebcam"", ""AllowKeyboard"", ""AllowMouse"", ""AllowActiveWindow"", ""AllowRunningPrograms"", ""IdleThresholdMinutes"", ""BlurScreenshots"", ""ExtraProperties"", ""ConcurrencyStamp"", ""CreationTime"")
                VALUES (@projId, @wsId, 'Base Project', '#112233', 10, 0, 1, 0, 1, 1, 1, 1, 5, 0, '{}', 'stamp2', @cTime);
            ";
            cmd.Parameters.Add(new SqliteParameter("@wsId", wsId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@projId", projId.ToString().ToUpperInvariant()));
            cmd.Parameters.Add(new SqliteParameter("@cTime", cTime));
            await cmd.ExecuteNonQueryAsync();
        }

        // 3. Migrate to current migration (20261021120000_AddProjectTasks)
        using (var context = new TrackerDbContext(sqliteOptions))
        {
            var migrator = context.GetService<IMigrator>();
            await migrator.MigrateAsync("20261021120000_AddProjectTasks");
        }

        // 4. Verify table and indexes exist
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'AppProjectTasks';";
            var tableCount = (long)(await cmd.ExecuteScalarAsync())!;
            tableCount.ShouldBe(1L);

            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'IX_AppProjectTasks_WorkspaceId_ProjectId';";
            var indexCount1 = (long)(await cmd.ExecuteScalarAsync())!;
            indexCount1.ShouldBe(1L);

            cmd.CommandText = "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'IX_AppProjectTasks_WorkspaceId_ProjectId_IsCompleted';";
            var indexCount2 = (long)(await cmd.ExecuteScalarAsync())!;
            indexCount2.ShouldBe(1L);
        }

        // 5. Verify existing baseline data survived intact
        await using (var cmd = connection.CreateCommand())
        {
            cmd.CommandText = "SELECT \"Title\" FROM \"AppProjects\" WHERE \"Id\" = @projId;";
            cmd.Parameters.Add(new SqliteParameter("@projId", projId.ToString().ToUpperInvariant()));
            await using var reader = await cmd.ExecuteReaderAsync();
            reader.Read().ShouldBeTrue();
            reader.GetString(0).ShouldBe("Base Project");
        }

        // 6. Verify production TrackerDbContext can persist and query ProjectTask
        var taskId = Guid.NewGuid();
        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var task = new ProjectTask(taskId, wsId, projId, "First Test Task", "Some description", false);
            await context.ProjectTasks.AddAsync(task);
            await context.SaveChangesAsync();
        }

        using (var context = new TrackerDbContext(sqliteOptions) { LazyServiceProvider = GetRequiredService<IAbpLazyServiceProvider>() })
        {
            var loaded = await context.ProjectTasks.IgnoreQueryFilters().FirstOrDefaultAsync(t => t.Id == taskId);
            loaded.ShouldNotBeNull();
            loaded.Title.ShouldBe("First Test Task");
            loaded.ProjectId.ShouldBe(projId);
            loaded.WorkspaceId.ShouldBe(wsId);
            loaded.IsCompleted.ShouldBeFalse();
        }
    }
}
