using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AgentDataV2Followup : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            // Data steps first, on PostgreSQL only (the SQLite test database is created from the model and never
            // migrated): they bring existing rows in line with the constraints added below. Each one is safe on existing
            // data and on a re-run; none of them can be reversed by Down().
            if (migrationBuilder.IsNpgsql())
            {
                BoundProjectColorsAndDescriptions(migrationBuilder);
                RemoveMembershipsOfDeletedProjects(migrationBuilder);
                RemoveDuplicateMemberships(migrationBuilder);
                AlignMembershipRatesWithHistory(migrationBuilder);
                RevokeUnbackedManagerRoles(migrationBuilder);
                RemoveDuplicateHostUploads(migrationBuilder);
                RenameDuplicateWorkspaceNames(migrationBuilder);
                RestartEndedTrials(migrationBuilder);
                ScrubSensitiveAuditParameters(migrationBuilder);
                BackfillIdleSeconds(migrationBuilder);
            }

            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities");

            migrationBuilder.DropIndex(
                name: "IX_AbpTenants_NormalizedName",
                table: "AbpTenants");

            migrationBuilder.AlterColumn<string>(
                name: "Description",
                table: "AppProjects",
                type: "character varying(2048)",
                maxLength: 2048,
                nullable: true,
                oldClrType: typeof(string),
                oldType: "text",
                oldNullable: true);

            migrationBuilder.AlterColumn<string>(
                name: "Color",
                table: "AppProjects",
                type: "character varying(7)",
                maxLength: 7,
                nullable: false,
                oldClrType: typeof(string),
                oldType: "text");

            migrationBuilder.CreateIndex(
                name: "IX_AppProjectMembers_TenantId_ProjectId_UserId",
                table: "AppProjectMembers",
                columns: new[] { "TenantId", "ProjectId", "UserId" },
                unique: true)
                .Annotation("Npgsql:NullsDistinct", false);

            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities",
                columns: new[] { "TenantId", "UserId", "ClientActivityId" },
                unique: true)
                .Annotation("Npgsql:NullsDistinct", false);

            migrationBuilder.CreateIndex(
                name: "IX_AbpTenants_NormalizedName",
                table: "AbpTenants",
                column: "NormalizedName",
                unique: true,
                filter: "\"IsDeleted\" = false");
        }

        /// <summary>A project colour is a "#RRGGBB" hex colour (anything else becomes the default colour) and a
        /// description holds at most 2048 characters, before the columns are bounded to that.</summary>
        private static void BoundProjectColorsAndDescriptions(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AppProjects"
                SET "Color" = '#006bff'
                WHERE "Color" IS NULL OR "Color" !~ '^#[0-9a-fA-F]{6}$';
                """);

            migrationBuilder.Sql("""
                UPDATE "AppProjects"
                SET "Description" = left("Description", 2048)
                WHERE length("Description") > 2048;
                """);
        }

        /// <summary>Deleting a project now removes its memberships; memberships of projects deleted before that are
        /// removed here. The members' rate history (AppProjectMemberRates) is kept for payroll of past time.</summary>
        private static void RemoveMembershipsOfDeletedProjects(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                DELETE FROM "AppProjectMembers" AS m
                WHERE NOT EXISTS (
                    SELECT 1 FROM "AppProjects" AS p
                    WHERE p."Id" = m."ProjectId" AND NOT p."IsDeleted");
                """);
        }

        /// <summary>A person joins a project once. Of memberships duplicated by concurrent adds, the oldest stays (the one
        /// AgentDataV2 seeded the rate history from).</summary>
        private static void RemoveDuplicateMemberships(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                DELETE FROM "AppProjectMembers" AS m
                USING (
                    SELECT "Id",
                           row_number() OVER (PARTITION BY "TenantId", "ProjectId", "UserId" ORDER BY "CreationTime", "Id") AS n
                    FROM "AppProjectMembers"
                ) AS d
                WHERE m."Id" = d."Id" AND d.n > 1;
                """);
        }

        /// <summary>A membership's rate is its member's latest recorded rate on the project — what payroll prices new
        /// time with, and what a later rate change is compared against. Membership and history already agree after a
        /// fresh upgrade; this settles any duplicate whose rate the history was seeded from differently.</summary>
        private static void AlignMembershipRatesWithHistory(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AppProjectMembers" AS m
                SET "HourlyRate" = latest."HourlyRate"
                FROM (
                    SELECT DISTINCT ON ("TenantId", "ProjectId", "UserId") "TenantId", "ProjectId", "UserId", "HourlyRate"
                    FROM "AppProjectMemberRates"
                    ORDER BY "TenantId", "ProjectId", "UserId", "EffectiveFrom" DESC, "CreationTime" DESC, "Id" DESC
                ) AS latest
                WHERE latest."TenantId" IS NOT DISTINCT FROM m."TenantId"
                  AND latest."ProjectId" = m."ProjectId"
                  AND latest."UserId" = m."UserId"
                  AND m."HourlyRate" <> latest."HourlyRate";
                """);
        }

        /// <summary>The workspace-wide "manager" role comes with an Admin/Manager membership and goes with the last one
        /// (ManagerRoleSynchronizer). Before that rule existed, removing a member and deleting a project left the role
        /// behind, with its access to every member's activity, screenshots and webcam frames: the role is taken from
        /// everyone who no longer holds such a membership on a project that still exists.</summary>
        private static void RevokeUnbackedManagerRoles(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                DELETE FROM "AbpUserRoles" AS ur
                USING "AbpRoles" AS r
                WHERE r."Id" = ur."RoleId"
                  AND r."NormalizedName" = 'MANAGER'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM "AppProjectMembers" AS m
                      JOIN "AppProjects" AS p ON p."Id" = m."ProjectId"
                      WHERE m."UserId" = ur."UserId"
                        AND NOT p."IsDeleted"
                        AND lower(btrim(m."Role", E' \t\r\n')) IN ('admin', 'manager'));
                """);
        }

        /// <summary>The upload index now treats a null TenantId as one value, so a host user's block stored twice by
        /// concurrent retries (same member, same client id: the same block) keeps only its first copy. Workspace blocks
        /// were already unique per member; their captures go with the removed copy (cascade).</summary>
        private static void RemoveDuplicateHostUploads(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                DELETE FROM "AppActivities" AS a
                USING (
                    SELECT "Id",
                           row_number() OVER (PARTITION BY "UserId", "ClientActivityId" ORDER BY "CreationTime", "Id") AS n
                    FROM "AppActivities"
                    WHERE "TenantId" IS NULL
                ) AS d
                WHERE a."Id" = d."Id" AND d.n > 1;
                """);
        }

        /// <summary>Two live workspaces never share a name: of workspaces that do (concurrent sign-ups), the oldest keeps
        /// it and each later one gets "-" and the first 8 characters of its id appended (within the 64-character limit).</summary>
        private static void RenameDuplicateWorkspaceNames(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AbpTenants" AS t
                SET "Name" = left(t."Name", 55) || '-' || left(t."Id"::text, 8),
                    "NormalizedName" = left(t."NormalizedName", 55) || '-' || upper(left(t."Id"::text, 8))
                FROM (
                    SELECT "Id",
                           row_number() OVER (PARTITION BY "NormalizedName" ORDER BY "CreationTime", "Id") AS n
                    FROM "AbpTenants"
                    WHERE NOT "IsDeleted"
                ) AS d
                WHERE t."Id" = d."Id" AND d.n > 1;
                """);
        }

        /// <summary>
        /// Subscriptions are enforced from now on (a workspace whose trial ended can no longer track time or grow its
        /// team until the host activates it), while until now an ended trial (past_due, nothing ever invoiced) simply
        /// kept working — the host had no way to activate it. Rather than cutting all those workspaces off without
        /// notice at the upgrade, each gets a new 14-day trial from now: its owners are reminded three days before it
        /// ends and told when it has ended (the hourly subscription sweep), and the host can activate the paying ones in
        /// the meantime. Workspaces with an unpaid invoice, deleted workspaces and every other status are left as they are.
        /// </summary>
        private static void RestartEndedTrials(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AppSubscriptions" AS s
                SET "Status" = 'trialing',
                    "TrialEndsAt" = (now() AT TIME ZONE 'UTC') + INTERVAL '14 days'
                WHERE s."Status" = 'past_due'
                  AND NOT s."IsDeleted"
                  AND EXISTS (
                      SELECT 1 FROM "AbpTenants" AS t
                      WHERE t."Id" = s."TenantId" AND NOT t."IsDeleted")
                  AND NOT EXISTS (
                      SELECT 1 FROM "AppInvoices" AS i
                      WHERE i."TenantId" = s."TenantId"
                        AND NOT i."IsDeleted"
                        AND i."Status" <> 'paid');
                """);
        }

        /// <summary>Audit rows no longer record secrets or capture payloads; earlier rows lose their parameters too: the
        /// owner's plaintext password of every workspace sign-up, and the window titles, timelines and captures of every
        /// block upload.</summary>
        private static void ScrubSensitiveAuditParameters(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.Sql("""
                UPDATE "AbpAuditLogActions"
                SET "Parameters" = '{}'
                WHERE "ServiceName" LIKE '%WorkspaceAppService%' AND "MethodName" = 'RegisterAsync';
                """);

            migrationBuilder.Sql("""
                UPDATE "AbpAuditLogActions"
                SET "Parameters" = '{}'
                WHERE "ServiceName" LIKE '%ActivityAppService%' AND "MethodName" = 'CreateAsync';
                """);
        }

        /// <summary>
        /// Blocks stored before idle time was recorded get it from their timeline, by the rule the server applies to an
        /// upload without idleSeconds (shared contract §1, ActivityMinuteBuckets): the block length rounded to whole
        /// seconds, buckets of 60 s with the last one absorbing the remainder (at least one bucket), and idle time = the
        /// summed length of the buckets marked not active (a bucket listed twice counts once, as its first entry), at
        /// most the block's length. Like on upload, only for projects that measure keyboard or mouse input; a block
        /// without a timeline (or whose timeline cannot be read) keeps no idle time.
        /// </summary>
        private static void BackfillIdleSeconds(MigrationBuilder migrationBuilder)
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
                SET "IdleSeconds" = idle.seconds
                FROM (
                    SELECT blocks."Id",
                           LEAST(blocks.span, SUM(
                               CASE
                                   WHEN buckets.active OR buckets.minute < 0 OR buckets.minute >= blocks.bucket_count THEN 0
                                   WHEN buckets.minute < blocks.bucket_count - 1 THEN 60
                                   ELSE blocks.span - 60 * (blocks.bucket_count - 1)
                               END)) AS seconds
                    FROM (
                        SELECT x."Id", s.span, GREATEST(1, (s.span + 30) / 60) AS bucket_count,
                               pg_temp.dosi_try_json_array(x."TimelineJson") AS doc
                        FROM "AppActivities" AS x
                        JOIN "AppProjects" AS p ON p."Id" = x."ProjectId"
                        CROSS JOIN LATERAL (
                            SELECT round(EXTRACT(EPOCH FROM (x."EndedAt" - x."StartedAt")))::integer AS span
                        ) AS s
                        WHERE x."IdleSeconds" IS NULL
                          AND x."TimelineJson" <> '[]'
                          AND (p."AllowKeyboard" OR p."AllowMouse")
                          AND s.span > 0
                    ) AS blocks
                    CROSS JOIN LATERAL (
                        SELECT DISTINCT ON (entries.minute) entries.minute, entries.active
                        FROM (
                            SELECT (t.e ->> 'Minute')::integer AS minute,
                                   CASE WHEN jsonb_typeof(t.e -> 'Active') = 'boolean' THEN (t.e -> 'Active')::boolean ELSE false END AS active,
                                   t.n
                            FROM jsonb_array_elements(blocks.doc) WITH ORDINALITY AS t(e, n)
                            WHERE jsonb_typeof(t.e) = 'object'
                              AND jsonb_typeof(t.e -> 'Minute') = 'number'
                              AND (t.e ->> 'Minute') ~ '^-?[0-9]{1,9}$'
                        ) AS entries
                        ORDER BY entries.minute, entries.n
                    ) AS buckets
                    WHERE blocks.doc IS NOT NULL
                    GROUP BY blocks."Id", blocks.span
                ) AS idle
                WHERE a."Id" = idle."Id";
                """);

            migrationBuilder.Sql("DROP FUNCTION pg_temp.dosi_try_json_array(text);");
        }

        /// <inheritdoc />
        /// <remarks>Reverses the schema changes only. The data steps of Up() cannot be undone: removed duplicate and
        /// orphaned rows, aligned rates, revoked manager roles, renamed workspaces, restarted trials, scrubbed audit
        /// parameters, bounded colours and descriptions and derived idle time stay as they are.</remarks>
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppProjectMembers_TenantId_ProjectId_UserId",
                table: "AppProjectMembers");

            migrationBuilder.DropIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities");

            migrationBuilder.DropIndex(
                name: "IX_AbpTenants_NormalizedName",
                table: "AbpTenants");

            migrationBuilder.AlterColumn<string>(
                name: "Description",
                table: "AppProjects",
                type: "text",
                nullable: true,
                oldClrType: typeof(string),
                oldType: "character varying(2048)",
                oldMaxLength: 2048,
                oldNullable: true);

            migrationBuilder.AlterColumn<string>(
                name: "Color",
                table: "AppProjects",
                type: "text",
                nullable: false,
                oldClrType: typeof(string),
                oldType: "character varying(7)",
                oldMaxLength: 7);

            migrationBuilder.CreateIndex(
                name: "IX_AppActivities_TenantId_UserId_ClientActivityId",
                table: "AppActivities",
                columns: new[] { "TenantId", "UserId", "ClientActivityId" },
                unique: true);

            migrationBuilder.CreateIndex(
                name: "IX_AbpTenants_NormalizedName",
                table: "AbpTenants",
                column: "NormalizedName");
        }
    }
}
