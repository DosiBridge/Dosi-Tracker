using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Migrations;
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
/// NullableActivityInputCounts: a block's mouse clicks and keystrokes may be "not measured" (null) instead of a
/// fabricated 0. The migration only relaxes the two columns — existing counts survive an upgrade unchanged — and the
/// tracked snapshot matches the runtime model, so no model change is left without a migration.
/// </summary>
public class NullableActivityInputCountsMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private const string PreviousMigration = "20261021120000_AddProjectTasks";

    private static IReadOnlyList<MigrationOperation> GetOperations(string method)
    {
        var builder = new MigrationBuilder("Npgsql.EntityFrameworkCore.PostgreSQL");
        typeof(NullableActivityInputCounts)
            .GetMethod(method, BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(new NullableActivityInputCounts(), new object[] { builder });
        return builder.Operations;
    }

    private static TrackerDbContext NewNpgsqlContext() => new(new DbContextOptionsBuilder<TrackerDbContext>()
        .UseNpgsql("Host=localhost;Database=dummy;")
        .Options);

    [Fact]
    public void Migration_Should_Follow_The_Previous_One_And_Target_The_Tracker_Context()
    {
        var attribute = typeof(NullableActivityInputCounts).GetCustomAttribute<MigrationAttribute>().ShouldNotBeNull();
        attribute.Id.ShouldEndWith("_NullableActivityInputCounts");
        string.CompareOrdinal(attribute.Id, PreviousMigration).ShouldBeGreaterThan(0);

        typeof(NullableActivityInputCounts).GetCustomAttribute<DbContextAttribute>()!.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Should_Only_Make_The_Two_Count_Columns_Nullable_And_Down_Should_Restore_Them()
    {
        var up = GetOperations("Up");
        up.Count.ShouldBe(2);
        foreach (var column in new[] { nameof(Activity.MouseClicks), nameof(Activity.KeyboardHits) })
        {
            var alter = up.OfType<AlterColumnOperation>().Single(o => o.Name == column);
            alter.Table.ShouldBe("AppActivities");
            alter.IsNullable.ShouldBeTrue();
            alter.OldColumn.IsNullable.ShouldBeFalse();
        }

        var down = GetOperations("Down").OfType<AlterColumnOperation>().ToList();
        down.Count.ShouldBe(2);
        down.ShouldAllBe(o => !o.IsNullable && Equals(o.DefaultValue, 0));
    }

    [Fact]
    public void Generated_PostgreSql_Ddl_Should_Drop_The_Not_Null_Constraints()
    {
        using var context = NewNpgsqlContext();
        var ddl = string.Join("\n", context.GetService<IMigrationsSqlGenerator>().Generate(GetOperations("Up")).Select(c => c.CommandText));

        ddl.ShouldContain("ALTER TABLE \"AppActivities\" ALTER COLUMN \"MouseClicks\" DROP NOT NULL");
        ddl.ShouldContain("ALTER TABLE \"AppActivities\" ALTER COLUMN \"KeyboardHits\" DROP NOT NULL");
    }

    [Fact]
    public void Runtime_Model_And_Tracked_Snapshot_Should_Both_Have_Nullable_Counts()
    {
        using var context = NewNpgsqlContext();
        var entity = context.Model.FindEntityType(typeof(Activity)).ShouldNotBeNull();
        entity.FindProperty(nameof(Activity.MouseClicks))!.IsNullable.ShouldBeTrue();
        entity.FindProperty(nameof(Activity.KeyboardHits))!.IsNullable.ShouldBeTrue();

        var snapshot = new TrackerDbContextModelSnapshot().Model.FindEntityType("Dosi.Tracker.Activities.Activity").ShouldNotBeNull();
        snapshot.FindProperty(nameof(Activity.MouseClicks))!.IsNullable.ShouldBeTrue();
        snapshot.FindProperty(nameof(Activity.KeyboardHits))!.IsNullable.ShouldBeTrue();
    }

    [Fact]
    public void The_Tracked_Snapshot_Should_Have_No_Pending_Model_Changes()
    {
        using var context = NewNpgsqlContext();

        context.Database.HasPendingModelChanges().ShouldBeFalse();
    }

    [Fact]
    public async Task Upgrading_Should_Keep_Existing_Counts_And_Then_Store_Unmeasured_Blocks_As_Null()
    {
        await using var connection = new SqliteConnection("Data Source=:memory:");
        await connection.OpenAsync();
        var options = new DbContextOptionsBuilder<TrackerDbContext>().UseSqlite(connection).Options;
        var lazy = GetRequiredService<IAbpLazyServiceProvider>();

        using (var context = new TrackerDbContext(options))
        {
            await context.GetService<IMigrator>().MigrateAsync(PreviousMigration);
        }

        var workspaceId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var start = new DateTime(2026, 10, 1, 9, 0, 0, DateTimeKind.Utc);
        var measured = new Activity(Guid.NewGuid(), workspaceId, userId, projectId, Guid.NewGuid(), start, start.AddMinutes(10))
        {
            MouseClicks = 12,
            KeyboardHits = 0
        };

        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            await context.Activities.AddAsync(measured);
            await context.SaveChangesAsync();
        }

        using (var context = new TrackerDbContext(options))
        {
            await context.GetService<IMigrator>().MigrateAsync();
        }

        var unmeasured = new Activity(Guid.NewGuid(), workspaceId, userId, projectId, Guid.NewGuid(), start.AddMinutes(10), start.AddMinutes(20));
        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            await context.Activities.AddAsync(unmeasured);
            await context.SaveChangesAsync();
        }

        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            var rows = await context.Activities.IgnoreQueryFilters().AsNoTracking().ToListAsync();

            var kept = rows.Single(a => a.Id == measured.Id);
            kept.MouseClicks.ShouldBe(12);
            kept.KeyboardHits.ShouldBe(0); // a measured zero stays a zero

            var stored = rows.Single(a => a.Id == unmeasured.Id);
            stored.MouseClicks.ShouldBeNull();
            stored.KeyboardHits.ShouldBeNull();
        }
    }
}
