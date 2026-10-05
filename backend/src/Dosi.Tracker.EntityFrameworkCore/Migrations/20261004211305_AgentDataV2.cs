using System;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AgentDataV2 : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_ClientActivityId",
                table: "AppActivities");

            migrationBuilder.AddColumn<DateTime>(
                name: "ArchivedAt",
                table: "AppProjects",
                type: "timestamp without time zone",
                nullable: true);

            migrationBuilder.AddColumn<bool>(
                name: "BlurScreenshots",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: false);

            migrationBuilder.AddColumn<int>(
                name: "IdleThresholdMinutes",
                table: "AppProjects",
                type: "integer",
                nullable: false,
                defaultValue: 5); // Existing projects get the default threshold, never 0 (= idle detection off).

            migrationBuilder.AddColumn<DateTime>(
                name: "BillingPeriodStart",
                table: "AppInvoices",
                type: "timestamp without time zone",
                nullable: true);

            migrationBuilder.AlterColumn<int>(
                name: "Productivity",
                table: "AppActivities",
                type: "integer",
                nullable: true,
                oldClrType: typeof(int),
                oldType: "integer");

            migrationBuilder.AddColumn<string>(
                name: "AgentVersion",
                table: "AppActivities",
                type: "character varying(32)",
                maxLength: 32,
                nullable: true);

            migrationBuilder.AddColumn<int>(
                name: "ClockSkewSeconds",
                table: "AppActivities",
                type: "integer",
                nullable: true);

            migrationBuilder.AddColumn<Guid>(
                name: "DeviceId",
                table: "AppActivities",
                type: "uuid",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "DeviceName",
                table: "AppActivities",
                type: "character varying(128)",
                maxLength: 128,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "EndReason",
                table: "AppActivities",
                type: "character varying(16)",
                maxLength: 16,
                nullable: true);

            migrationBuilder.AddColumn<int>(
                name: "IdleSeconds",
                table: "AppActivities",
                type: "integer",
                nullable: true);

            migrationBuilder.AddColumn<int>(
                name: "InjectedInputEvents",
                table: "AppActivities",
                type: "integer",
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "OsVersion",
                table: "AppActivities",
                type: "character varying(64)",
                maxLength: 64,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "Platform",
                table: "AppActivities",
                type: "character varying(16)",
                maxLength: 16,
                nullable: true);

            migrationBuilder.AddColumn<string>(
                name: "TimeZone",
                table: "AppActivities",
                type: "character varying(64)",
                maxLength: 64,
                nullable: true);

            migrationBuilder.AddColumn<int>(
                name: "UtcOffsetMinutes",
                table: "AppActivities",
                type: "integer",
                nullable: true);

            migrationBuilder.CreateTable(
                name: "AppAgentPresences",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    TenantId = table.Column<Guid>(type: "uuid", nullable: true),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    DeviceId = table.Column<Guid>(type: "uuid", nullable: false),
                    DeviceName = table.Column<string>(type: "character varying(128)", maxLength: 128, nullable: true),
                    Platform = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: true),
                    OsVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    AgentVersion = table.Column<string>(type: "character varying(32)", maxLength: 32, nullable: true),
                    Status = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    ProjectId = table.Column<Guid>(type: "uuid", nullable: true),
                    StatusSince = table.Column<DateTime>(type: "timestamp without time zone", nullable: true),
                    LastSeenAt = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CurrentApp = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: true),
                    TimeZone = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    UtcOffsetMinutes = table.Column<int>(type: "integer", nullable: true),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppAgentPresences", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "AppProjectMemberRates",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    TenantId = table.Column<Guid>(type: "uuid", nullable: true),
                    ProjectId = table.Column<Guid>(type: "uuid", nullable: false),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    HourlyRate = table.Column<decimal>(type: "numeric", nullable: false),
                    EffectiveFrom = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppProjectMemberRates", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_AppInvoices_TenantId_BillingPeriodStart",
                table: "AppInvoices",
                columns: new[] { "TenantId", "BillingPeriodStart" },
                unique: true,
                filter: "\"BillingPeriodStart\" IS NOT NULL AND \"IsDeleted\" = false");

            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_ClientActivityId",
                table: "AppActivities",
                columns: new[] { "TenantId", "ClientActivityId" });

            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_StartedAt",
                table: "AppActivities",
                columns: new[] { "TenantId", "StartedAt" },
                descending: new[] { false, true });

            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities",
                columns: new[] { "TenantId", "UserId", "ClientActivityId" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_AppAgentPresences_TenantId_UserId_DeviceId",
                table: "AppAgentPresences",
                columns: new[] { "TenantId", "UserId", "DeviceId" },
                unique: true)
                .Annotation("Npgsql:NullsDistinct", false);

            migrationBuilder.CreateIndex(
                name: "IX_AppProjectMemberRates_TenantId_ProjectId_UserId_EffectiveFr~",
                table: "AppProjectMemberRates",
                columns: new[] { "TenantId", "ProjectId", "UserId", "EffectiveFrom" });

            // Data steps, on PostgreSQL only (the SQLite test database is created from the model and never migrated).
            // Each one is safe on existing data and on a re-run; none of them can be reversed by Down().
            if (migrationBuilder.IsNpgsql())
            {
                SeedRateHistory(migrationBuilder);
                MarkUnmeasuredProductivity(migrationBuilder);
                RemoveBackgroundWindowTitles(migrationBuilder);
                StampArchivedProjects(migrationBuilder);
            }
        }

        /// <summary>Every existing membership's rate becomes its first rate-history entry, effective since long
        /// before any tracked time, so payroll prices past work as before. One entry per (tenant, project, member): of
        /// a membership duplicated by concurrent adds, the one that AgentDataV2Followup keeps (the oldest, by creation
        /// time and then id), so payroll and the surviving membership agree on the rate.</summary>
        private static void SeedRateHistory(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                INSERT INTO "AppProjectMemberRates" ("Id", "TenantId", "ProjectId", "UserId", "HourlyRate", "EffectiveFrom", "CreationTime")
                SELECT gen_random_uuid(), m."TenantId", m."ProjectId", m."UserId", m."HourlyRate",
                       TIMESTAMP '2000-01-01 00:00:00', now() AT TIME ZONE 'UTC'
                FROM (
                    SELECT DISTINCT ON ("TenantId", "ProjectId", "UserId") "TenantId", "ProjectId", "UserId", "HourlyRate"
                    FROM "AppProjectMembers"
                    ORDER BY "TenantId", "ProjectId", "UserId", "CreationTime", "Id"
                ) AS m
                WHERE NOT EXISTS (
                    SELECT 1 FROM "AppProjectMemberRates" AS r
                    WHERE r."TenantId" IS NOT DISTINCT FROM m."TenantId"
                      AND r."ProjectId" = m."ProjectId"
                      AND r."UserId" = m."UserId");
                """);
        }

        /// <summary>
        /// Blocks whose activity was never measured become "not measured" (null) instead of dragging averages to 0%:
        /// <list type="bullet">
        /// <item>blocks from agents that never computed it (no timeline, input counted, yet 0%) — under the per-minute
        /// rule any input in a measured block gives at least 1%, so a genuinely measured block never matches;</item>
        /// <item>0% blocks on projects that measure neither keyboard nor mouse: the agents saw no input because they were
        /// told not to look (uploads to such projects are stored as null from now on). Like the idle-time backfill of
        /// AgentDataV2Followup this goes by the projects' current settings, the only ones recorded.</item>
        /// </list>
        /// </summary>
        private static void MarkUnmeasuredProductivity(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AppActivities"
                SET "Productivity" = NULL
                WHERE "Productivity" = 0
                  AND "TimelineJson" = '[]'
                  AND "KeyboardHits"::bigint + "MouseClicks"::bigint > 0;
                """);

            migrationBuilder.Sql("""
                UPDATE "AppActivities" AS a
                SET "Productivity" = NULL
                FROM "AppProjects" AS p
                WHERE p."Id" = a."ProjectId"
                  AND NOT p."AllowKeyboard"
                  AND NOT p."AllowMouse"
                  AND a."Productivity" = 0;
                """);
        }

        /// <summary>Titles of background windows are never kept (privacy): drops the WindowTitle key from every
        /// element of RunningProgramsJson. Rows whose value is not a valid JSON array (or cannot be read as jsonb)
        /// are left untouched. Rewritten rows take jsonb's canonical text form, which every reader parses alike.</summary>
        private static void RemoveBackgroundWindowTitles(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                CREATE FUNCTION pg_temp.dosi_try_json_array(value text) RETURNS jsonb
                    LANGUAGE plpgsql IMMUTABLE STRICT AS $$
                DECLARE
                    doc jsonb;
                BEGIN
                    doc := value::jsonb;
                    RETURN CASE WHEN jsonb_typeof(doc) = 'array' THEN doc END;
                EXCEPTION WHEN others THEN
                    RETURN NULL;
                END;
                $$;
                """);

            migrationBuilder.Sql("""
                UPDATE "AppActivities" AS a
                SET "RunningProgramsJson" = (
                    SELECT COALESCE(
                               jsonb_agg(CASE WHEN jsonb_typeof(t.e) = 'object' THEN t.e - 'WindowTitle' ELSE t.e END ORDER BY t.n),
                               '[]'::jsonb)::text
                    FROM jsonb_array_elements(p.doc) WITH ORDINALITY AS t(e, n))
                FROM (
                    SELECT "Id", pg_temp.dosi_try_json_array("RunningProgramsJson") AS doc
                    FROM "AppActivities"
                    WHERE "RunningProgramsJson" <> '[]'
                      AND strpos("RunningProgramsJson", '"WindowTitle"') > 0
                ) AS p
                WHERE a."Id" = p."Id"
                  AND p.doc IS NOT NULL
                  AND EXISTS (
                      SELECT 1 FROM jsonb_array_elements(p.doc) AS x(e)
                      WHERE jsonb_typeof(x.e) = 'object' AND x.e -> 'WindowTitle' IS NOT NULL);
                """);

            migrationBuilder.Sql("DROP FUNCTION pg_temp.dosi_try_json_array(text);");
        }

        /// <summary>Projects archived before ArchivedAt existed get their best-known archive moment: the last
        /// modification (the archive action itself, unless edited since), else their creation.</summary>
        private static void StampArchivedProjects(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AppProjects"
                SET "ArchivedAt" = COALESCE("LastModificationTime", "CreationTime")
                WHERE "IsArchived" AND "ArchivedAt" IS NULL;
                """);
        }

        /// <inheritdoc />
        /// <remarks>Reverses the schema changes only. The data steps of Up() cannot be undone: the removed
        /// background window titles are gone, and productivity marked "not measured" comes back as 0.</remarks>
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "AppAgentPresences");

            migrationBuilder.DropTable(
                name: "AppProjectMemberRates");

            migrationBuilder.DropIndex(
                name: "IX_AppInvoices_TenantId_BillingPeriodStart",
                table: "AppInvoices");

            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_ClientActivityId",
                table: "AppActivities");

            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_StartedAt",
                table: "AppActivities");

            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "ArchivedAt",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "BlurScreenshots",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "IdleThresholdMinutes",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "BillingPeriodStart",
                table: "AppInvoices");

            migrationBuilder.DropColumn(
                name: "AgentVersion",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "ClockSkewSeconds",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "DeviceId",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "DeviceName",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "EndReason",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "IdleSeconds",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "InjectedInputEvents",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "OsVersion",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "Platform",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "TimeZone",
                table: "AppActivities");

            migrationBuilder.DropColumn(
                name: "UtcOffsetMinutes",
                table: "AppActivities");

            // NOT NULL again: Npgsql first sets the "not measured" (null) rows to the default, 0.
            migrationBuilder.AlterColumn<int>(
                name: "Productivity",
                table: "AppActivities",
                type: "integer",
                nullable: false,
                defaultValue: 0,
                oldClrType: typeof(int),
                oldType: "integer",
                oldNullable: true);

            // Fails if, since Up(), two members of a workspace stored the same client id (the ingest service
            // refuses that, so only a concurrent race could have produced it).
            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_ClientActivityId",
                table: "AppActivities",
                columns: new[] { "TenantId", "ClientActivityId" },
                unique: true);
        }
    }
}
