using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AddAiAssist : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "AppAiConsentEvents",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    WorkspaceId = table.Column<Guid>(type: "uuid", nullable: false),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    Action = table.Column<string>(type: "character varying(24)", maxLength: 24, nullable: false),
                    DisclosureVersion = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    Flags = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppAiConsentEvents", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "AppAiUsageRecords",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    WorkspaceId = table.Column<Guid>(type: "uuid", nullable: false),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    FeatureId = table.Column<string>(type: "character varying(48)", maxLength: 48, nullable: false),
                    PromptVersion = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: false),
                    Audience = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    SubjectUserId = table.Column<Guid>(type: "uuid", nullable: true),
                    ProjectId = table.Column<Guid>(type: "uuid", nullable: true),
                    RangeFrom = table.Column<DateTime>(type: "timestamp without time zone", nullable: true),
                    RangeTo = table.Column<DateTime>(type: "timestamp without time zone", nullable: true),
                    DataCategories = table.Column<string>(type: "character varying(256)", maxLength: 256, nullable: false),
                    Provider = table.Column<string>(type: "character varying(32)", maxLength: 32, nullable: false),
                    Model = table.Column<string>(type: "character varying(128)", maxLength: 128, nullable: true),
                    Outcome = table.Column<string>(type: "character varying(24)", maxLength: 24, nullable: false),
                    ErrorCode = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    CacheHit = table.Column<bool>(type: "boolean", nullable: false),
                    IsRegenerate = table.Column<bool>(type: "boolean", nullable: false),
                    InputTokens = table.Column<int>(type: "integer", nullable: true),
                    OutputTokens = table.Column<int>(type: "integer", nullable: true),
                    CachedInputTokens = table.Column<int>(type: "integer", nullable: true),
                    ReasoningTokens = table.Column<int>(type: "integer", nullable: true),
                    EstimatedCostUsd = table.Column<decimal>(type: "numeric(18,6)", precision: 18, scale: 6, nullable: true),
                    LatencyMs = table.Column<int>(type: "integer", nullable: true),
                    ToolCalls = table.Column<string>(type: "character varying(512)", maxLength: 512, nullable: true),
                    FeedbackRating = table.Column<string>(type: "character varying(8)", maxLength: 8, nullable: true),
                    FeedbackReason = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: true),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppAiUsageRecords", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_AppAiConsentEvents_WorkspaceId_CreationTime",
                table: "AppAiConsentEvents",
                columns: new[] { "WorkspaceId", "CreationTime" });

            migrationBuilder.CreateIndex(
                name: "IX_AppAiUsageRecords_CreationTime",
                table: "AppAiUsageRecords",
                column: "CreationTime");

            migrationBuilder.CreateIndex(
                name: "IX_AppAiUsageRecords_WorkspaceId_CreationTime",
                table: "AppAiUsageRecords",
                columns: new[] { "WorkspaceId", "CreationTime" });

            migrationBuilder.CreateIndex(
                name: "IX_AppAiUsageRecords_WorkspaceId_UserId_CreationTime",
                table: "AppAiUsageRecords",
                columns: new[] { "WorkspaceId", "UserId", "CreationTime" });
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "AppAiConsentEvents");

            migrationBuilder.DropTable(
                name: "AppAiUsageRecords");
        }
    }
}
