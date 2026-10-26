using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Migrations;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.DataProtection.EntityFrameworkCore;
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
/// ErasureAndDataProtection: workspaces can be soft-deleted (and record when their data was purged), a deleted
/// workspace's name no longer blocks a new one, deletions of tracked data and pending account erasures get their
/// tables, and Data Protection keys are kept in the database. Existing workspaces survive the upgrade, not deleted.
/// </summary>
public class ErasureAndDataProtectionMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private const string PreviousMigration = "20261025195615_NullableActivityInputCounts";

    private static IReadOnlyList<MigrationOperation> GetOperations(string method)
    {
        var builder = new MigrationBuilder("Npgsql.EntityFrameworkCore.PostgreSQL");
        typeof(ErasureAndDataProtection)
            .GetMethod(method, BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(new ErasureAndDataProtection(), new object[] { builder });
        return builder.Operations;
    }

    private static TrackerDbContext NewNpgsqlContext() => new(new DbContextOptionsBuilder<TrackerDbContext>()
        .UseNpgsql("Host=localhost;Database=dummy;")
        .Options);

    [Fact]
    public void Migration_Should_Follow_The_Previous_One_And_Target_The_Tracker_Context()
    {
        var attribute = typeof(ErasureAndDataProtection).GetCustomAttribute<MigrationAttribute>().ShouldNotBeNull();
        attribute.Id.ShouldEndWith("_ErasureAndDataProtection");
        string.CompareOrdinal(attribute.Id, PreviousMigration).ShouldBeGreaterThan(0);
        typeof(ErasureAndDataProtection).GetCustomAttribute<DbContextAttribute>()!.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Should_Add_The_Soft_Delete_Columns_The_Three_Tables_And_A_Name_Index_For_Live_Workspaces()
    {
        var up = GetOperations("Up");

        up.OfType<AddColumnOperation>().Where(o => o.Table == "AppWorkspaces").Select(o => o.Name)
            .ShouldBe(new[] { "DeleterId", "DeletionTime", "IsDeleted", "PurgedTime" }, ignoreOrder: true);
        up.OfType<AddColumnOperation>().Single(o => o.Name == "IsDeleted").DefaultValue.ShouldBe(false);
        up.OfType<CreateTableOperation>().Select(o => o.Name)
            .ShouldBe(new[] { "AppAccountErasures", "AppActivityErasures", "AppDataProtectionKeys" }, ignoreOrder: true);

        var nameIndex = up.OfType<CreateIndexOperation>().Single(o => o.Name == "IX_AppWorkspaces_NormalizedName");
        nameIndex.IsUnique.ShouldBeTrue();
        nameIndex.Filter.ShouldBe("\"IsDeleted\" = false");
        up.OfType<DropIndexOperation>().ShouldHaveSingleItem().Name.ShouldBe("IX_AppWorkspaces_NormalizedName");

        var down = GetOperations("Down");
        down.OfType<DropTableOperation>().Count().ShouldBe(3);
        down.OfType<DropColumnOperation>().Count().ShouldBe(4);
        down.OfType<CreateIndexOperation>().Single(o => o.Name == "IX_AppWorkspaces_NormalizedName").Filter.ShouldBeNull();
    }

    [Fact]
    public void Generated_PostgreSql_Ddl_Should_Make_The_Name_Unique_Among_Live_Workspaces_Only()
    {
        using var context = NewNpgsqlContext();
        var ddl = string.Join("\n", context.GetService<IMigrationsSqlGenerator>().Generate(GetOperations("Up")).Select(c => c.CommandText));

        ddl.ShouldContain("CREATE UNIQUE INDEX \"IX_AppWorkspaces_NormalizedName\" ON \"AppWorkspaces\" (\"NormalizedName\") WHERE \"IsDeleted\" = false;");
        ddl.ShouldContain("ALTER TABLE \"AppWorkspaces\" ADD \"IsDeleted\" boolean NOT NULL DEFAULT FALSE;");
        ddl.ShouldContain("CREATE TABLE \"AppDataProtectionKeys\"");
        ddl.ShouldContain("CREATE INDEX \"IX_AppActivityErasures_WorkspaceId_UserId_ClientActivityId\"");
    }

    [Fact]
    public void Runtime_Model_And_Tracked_Snapshot_Should_Agree()
    {
        using var context = NewNpgsqlContext();
        context.Model.FindEntityType(typeof(Workspace))!.FindProperty(nameof(Workspace.PurgedTime)).ShouldNotBeNull();
        context.Model.FindEntityType(typeof(ActivityErasure))!.GetTableName().ShouldBe("AppActivityErasures");
        context.Model.FindEntityType(typeof(AccountErasure))!.GetTableName().ShouldBe("AppAccountErasures");
        context.Model.FindEntityType(typeof(DataProtectionKey))!.GetTableName().ShouldBe("AppDataProtectionKeys");

        var snapshot = new TrackerDbContextModelSnapshot().Model;
        snapshot.FindEntityType("Dosi.Tracker.Workspaces.Workspace")!.FindProperty(nameof(Workspace.IsDeleted)).ShouldNotBeNull();
        snapshot.FindEntityType("Dosi.Tracker.Activities.ActivityErasure").ShouldNotBeNull();

        context.Database.HasPendingModelChanges().ShouldBeFalse();
    }

    [Fact]
    public async Task Upgrading_Should_Keep_Workspaces_Live_And_Free_The_Name_Of_A_Deleted_One()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        var options = new DbContextOptionsBuilder<TrackerDbContext>().UseSqlite(connection).Options;
        var lazy = GetRequiredService<IAbpLazyServiceProvider>();

        using (var context = new TrackerDbContext(options))
        {
            await context.GetService<IMigrator>().MigrateAsync(PreviousMigration);
        }

        var existingId = Guid.NewGuid();
        await using (var insert = connection.CreateCommand())
        {
            insert.CommandText = "INSERT INTO \"AppWorkspaces\" (\"Id\", \"Name\", \"NormalizedName\", \"IsArchived\", \"ExtraProperties\", \"ConcurrencyStamp\", \"CreationTime\") " +
                                 "VALUES (@id, 'Acme', 'ACME', 0, '{}', 'stamp', '2026-10-01 09:00:00');";
            insert.Parameters.Add(new SqliteParameter("@id", existingId.ToString().ToUpperInvariant()));
            await insert.ExecuteNonQueryAsync();
        }

        using (var context = new TrackerDbContext(options))
        {
            await context.GetService<IMigrator>().MigrateAsync();
        }

        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            var existing = await context.Workspaces.SingleAsync(w => w.Id == existingId);
            existing.IsDeleted.ShouldBeFalse();
            existing.PurgedTime.ShouldBeNull();

            // A second live workspace named the same is still refused by the database ...
            context.Workspaces.Add(new Workspace(Guid.NewGuid(), "acme"));
            await Should.ThrowAsync<DbUpdateException>(() => context.SaveChangesAsync());
        }

        // ... but once the first is (soft-)deleted, the name can be used again.
        await using (var delete = connection.CreateCommand())
        {
            delete.CommandText = "UPDATE \"AppWorkspaces\" SET \"IsDeleted\" = 1 WHERE \"Id\" = @id;";
            delete.Parameters.Add(new SqliteParameter("@id", existingId.ToString().ToUpperInvariant()));
            (await delete.ExecuteNonQueryAsync()).ShouldBe(1);
        }

        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            context.Workspaces.Add(new Workspace(Guid.NewGuid(), "ACME"));
            await context.SaveChangesAsync();

            (await context.Workspaces.IgnoreQueryFilters().CountAsync(w => w.NormalizedName == "ACME")).ShouldBe(2);
            (await context.Workspaces.CountAsync(w => w.NormalizedName == "ACME")).ShouldBe(1);
        }
    }
}
