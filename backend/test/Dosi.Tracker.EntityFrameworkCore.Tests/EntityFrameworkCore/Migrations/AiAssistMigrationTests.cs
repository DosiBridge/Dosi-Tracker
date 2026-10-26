using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist;
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
/// AddAiAssist: the AI usage ledger and the AI consent history get their tables and indexes (two new tables, nothing
/// else changed), and existing data survives the upgrade.
/// </summary>
public class AiAssistMigrationTests : TrackerEntityFrameworkCoreTestBase
{
    private const string PreviousMigration = "20261025222607_ErasureAndDataProtection";

    private static IReadOnlyList<MigrationOperation> GetOperations(string method)
    {
        var builder = new MigrationBuilder("Npgsql.EntityFrameworkCore.PostgreSQL");
        typeof(AddAiAssist)
            .GetMethod(method, BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(new AddAiAssist(), new object[] { builder });
        return builder.Operations;
    }

    private static TrackerDbContext NewNpgsqlContext() => new(new DbContextOptionsBuilder<TrackerDbContext>()
        .UseNpgsql("Host=localhost;Database=dummy;")
        .Options);

    [Fact]
    public void Migration_Should_Follow_The_Previous_One_And_Target_The_Tracker_Context()
    {
        var attribute = typeof(AddAiAssist).GetCustomAttribute<MigrationAttribute>().ShouldNotBeNull();
        attribute.Id.ShouldEndWith("_AddAiAssist");
        string.CompareOrdinal(attribute.Id, PreviousMigration).ShouldBeGreaterThan(0);
        typeof(AddAiAssist).GetCustomAttribute<DbContextAttribute>()!.ContextType.ShouldBe(typeof(TrackerDbContext));
    }

    [Fact]
    public void Up_Should_Create_Only_The_Two_Tables_With_Their_Indexes_And_Down_Should_Drop_Them()
    {
        var up = GetOperations("Up");

        up.Select(o => o.GetType()).Distinct().ShouldBe(new[] { typeof(CreateTableOperation), typeof(CreateIndexOperation) }, ignoreOrder: true);
        up.OfType<CreateTableOperation>().Select(o => o.Name).ShouldBe(new[] { "AppAiUsageRecords", "AppAiConsentEvents" }, ignoreOrder: true);

        var ledger = up.OfType<CreateTableOperation>().Single(o => o.Name == "AppAiUsageRecords");
        ledger.Columns.Select(c => c.Name).ShouldBe(new[]
        {
            "Id", "WorkspaceId", "UserId", "FeatureId", "PromptVersion", "Audience", "SubjectUserId", "ProjectId", "RangeFrom",
            "RangeTo", "DataCategories", "Provider", "Model", "Outcome", "ErrorCode", "CacheHit", "IsRegenerate", "InputTokens",
            "OutputTokens", "CachedInputTokens", "ReasoningTokens", "EstimatedCostUsd", "LatencyMs", "ToolCalls",
            "FeedbackRating", "FeedbackReason", "CreationTime", "CreatorId"
        }, ignoreOrder: true);
        // No content: nothing that could hold a prompt, an answer or a name.
        ledger.Columns.Where(c => c.ClrType == typeof(string)).ShouldAllBe(c => c.MaxLength != null && c.MaxLength <= 512);
        var cost = ledger.Columns.Single(c => c.Name == "EstimatedCostUsd");
        (cost.Precision, cost.Scale, cost.IsNullable).ShouldBe((18, 6, true));
        ledger.Columns.Single(c => c.Name == "FeatureId").MaxLength.ShouldBe(AiUsageRecord.MaxFeatureIdLength);
        ledger.Columns.Single(c => c.Name == "Outcome").MaxLength.ShouldBe(AiUsageRecord.MaxOutcomeLength);

        var consent = up.OfType<CreateTableOperation>().Single(o => o.Name == "AppAiConsentEvents");
        consent.Columns.Select(c => c.Name).ShouldBe(new[]
        {
            "Id", "WorkspaceId", "UserId", "Action", "DisclosureVersion", "Flags", "CreationTime", "CreatorId"
        }, ignoreOrder: true);

        up.OfType<CreateIndexOperation>().Select(o => (o.Table, string.Join(",", o.Columns))).ShouldBe(new[]
        {
            ("AppAiUsageRecords", "WorkspaceId,CreationTime"),
            ("AppAiUsageRecords", "WorkspaceId,UserId,CreationTime"),
            ("AppAiUsageRecords", "CreationTime"),
            ("AppAiConsentEvents", "WorkspaceId,CreationTime")
        }, ignoreOrder: true);

        var down = GetOperations("Down");
        down.ShouldAllBe(o => o is DropTableOperation);
        down.OfType<DropTableOperation>().Select(o => o.Name).ShouldBe(new[] { "AppAiUsageRecords", "AppAiConsentEvents" }, ignoreOrder: true);
    }

    [Fact]
    public void Generated_PostgreSql_Ddl_Should_Create_The_Tables_And_Indexes()
    {
        using var context = NewNpgsqlContext();
        var ddl = string.Join("\n", context.GetService<IMigrationsSqlGenerator>().Generate(GetOperations("Up")).Select(c => c.CommandText));

        ddl.ShouldContain("CREATE TABLE \"AppAiUsageRecords\"");
        ddl.ShouldContain("CREATE TABLE \"AppAiConsentEvents\"");
        ddl.ShouldContain("\"EstimatedCostUsd\" numeric(18,6)");
        ddl.ShouldContain("CREATE INDEX \"IX_AppAiUsageRecords_WorkspaceId_UserId_CreationTime\" ON \"AppAiUsageRecords\" (\"WorkspaceId\", \"UserId\", \"CreationTime\");");
        ddl.ShouldContain("CREATE INDEX \"IX_AppAiUsageRecords_WorkspaceId_CreationTime\" ON \"AppAiUsageRecords\" (\"WorkspaceId\", \"CreationTime\");");
        ddl.ShouldContain("CREATE INDEX \"IX_AppAiUsageRecords_CreationTime\" ON \"AppAiUsageRecords\" (\"CreationTime\");");
        ddl.ShouldContain("CREATE INDEX \"IX_AppAiConsentEvents_WorkspaceId_CreationTime\" ON \"AppAiConsentEvents\" (\"WorkspaceId\", \"CreationTime\");");
    }

    [Fact]
    public void Runtime_Model_And_Tracked_Snapshot_Should_Agree()
    {
        using var context = NewNpgsqlContext();
        context.Model.FindEntityType(typeof(AiUsageRecord))!.GetTableName().ShouldBe("AppAiUsageRecords");
        context.Model.FindEntityType(typeof(AiConsentEvent))!.GetTableName().ShouldBe("AppAiConsentEvents");

        var snapshot = new TrackerDbContextModelSnapshot().Model;
        snapshot.FindEntityType("Dosi.Tracker.AiAssist.AiUsageRecord").ShouldNotBeNull();
        snapshot.FindEntityType("Dosi.Tracker.AiAssist.AiConsentEvent").ShouldNotBeNull();

        context.Database.HasPendingModelChanges().ShouldBeFalse();
    }

    [Fact]
    public async Task Upgrading_Should_Keep_Existing_Data_And_Store_Ledger_And_Consent_Rows()
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
        await using (var insert = connection.CreateCommand())
        {
            insert.CommandText = "INSERT INTO \"AppWorkspaces\" (\"Id\", \"Name\", \"NormalizedName\", \"IsArchived\", \"IsDeleted\", \"ExtraProperties\", \"ConcurrencyStamp\", \"CreationTime\") " +
                                 "VALUES (@id, 'Acme', 'ACME', 0, 0, '{}', 'stamp', '2026-10-01 09:00:00');";
            insert.Parameters.Add(new SqliteParameter("@id", workspaceId.ToString().ToUpperInvariant()));
            await insert.ExecuteNonQueryAsync();
        }

        using (var context = new TrackerDbContext(options))
        {
            await context.GetService<IMigrator>().MigrateAsync();
        }

        var userId = Guid.NewGuid();
        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            (await context.Workspaces.IgnoreQueryFilters().SingleAsync(w => w.Id == workspaceId)).Name.ShouldBe("Acme");

            var record = new AiUsageRecord(Guid.NewGuid(), workspaceId, userId, AiFeatureIds.PeriodSummaryId, "period-summary.v1",
                AiUsageRecord.TeamAudience, AiOptions.ProviderOpenAi, "gpt-5.4-mini", AiUsageOutcomes.Generated, "durations,project-names");
            record.SetUsage(1200, 300, cachedInputTokens: 1000, estimatedCostUsd: 0.000123m);
            context.AiUsageRecords.Add(record);
            context.AiConsentEvents.Add(new AiConsentEvent(Guid.NewGuid(), workspaceId, userId, AiConsentEvent.EnabledAction, "1.0",
                "managers=1;selfService=1;windowTitles=0;notes=0"));
            await context.SaveChangesAsync();
        }

        using (var context = new TrackerDbContext(options) { LazyServiceProvider = lazy })
        {
            var stored = await context.AiUsageRecords.IgnoreQueryFilters().SingleAsync();
            stored.EstimatedCostUsd.ShouldBe(0.000123m);
            stored.InputTokens.ShouldBe(1200);
            stored.CalledProvider.ShouldBeTrue();
            (await context.AiConsentEvents.IgnoreQueryFilters().SingleAsync()).Action.ShouldBe(AiConsentEvent.EnabledAction);
        }
    }
}
