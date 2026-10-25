using System;
using Microsoft.EntityFrameworkCore.Migrations;
using Npgsql.EntityFrameworkCore.PostgreSQL.Metadata;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class ErasureAndDataProtection : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppWorkspaces_NormalizedName",
                table: "AppWorkspaces");

            migrationBuilder.AddColumn<Guid>(
                name: "DeleterId",
                table: "AppWorkspaces",
                type: "uuid",
                nullable: true);

            migrationBuilder.AddColumn<DateTime>(
                name: "DeletionTime",
                table: "AppWorkspaces",
                type: "timestamp without time zone",
                nullable: true);

            migrationBuilder.AddColumn<bool>(
                name: "IsDeleted",
                table: "AppWorkspaces",
                type: "boolean",
                nullable: false,
                defaultValue: false);

            migrationBuilder.AddColumn<DateTime>(
                name: "PurgedTime",
                table: "AppWorkspaces",
                type: "timestamp without time zone",
                nullable: true);

            migrationBuilder.CreateTable(
                name: "AppAccountErasures",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    CompletedTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: true),
                    ExtraProperties = table.Column<string>(type: "text", nullable: false),
                    ConcurrencyStamp = table.Column<string>(type: "character varying(40)", maxLength: 40, nullable: false),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppAccountErasures", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "AppActivityErasures",
                columns: table => new
                {
                    Id = table.Column<Guid>(type: "uuid", nullable: false),
                    WorkspaceId = table.Column<Guid>(type: "uuid", nullable: false),
                    ActivityId = table.Column<Guid>(type: "uuid", nullable: false),
                    UserId = table.Column<Guid>(type: "uuid", nullable: false),
                    ProjectId = table.Column<Guid>(type: "uuid", nullable: false),
                    ClientActivityId = table.Column<Guid>(type: "uuid", nullable: false),
                    StartedAt = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    EndedAt = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    Scope = table.Column<string>(type: "character varying(16)", maxLength: 16, nullable: false),
                    CaptureKinds = table.Column<string>(type: "character varying(64)", maxLength: 64, nullable: true),
                    CaptureCount = table.Column<int>(type: "integer", nullable: false),
                    Reason = table.Column<string>(type: "character varying(512)", maxLength: 512, nullable: true),
                    CreationTime = table.Column<DateTime>(type: "timestamp without time zone", nullable: false),
                    CreatorId = table.Column<Guid>(type: "uuid", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppActivityErasures", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "AppDataProtectionKeys",
                columns: table => new
                {
                    Id = table.Column<int>(type: "integer", nullable: false)
                        .Annotation("Npgsql:ValueGenerationStrategy", NpgsqlValueGenerationStrategy.IdentityByDefaultColumn),
                    FriendlyName = table.Column<string>(type: "text", nullable: true),
                    Xml = table.Column<string>(type: "text", nullable: true)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_AppDataProtectionKeys", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_AppWorkspaces_IsDeleted_PurgedTime",
                table: "AppWorkspaces",
                columns: new[] { "IsDeleted", "PurgedTime" });

            migrationBuilder.CreateIndex(
                name: "IX_AppWorkspaces_NormalizedName",
                table: "AppWorkspaces",
                column: "NormalizedName",
                unique: true,
                filter: "\"IsDeleted\" = false");

            migrationBuilder.CreateIndex(
                name: "IX_AppAccountErasures_CompletedTime",
                table: "AppAccountErasures",
                column: "CompletedTime");

            migrationBuilder.CreateIndex(
                name: "IX_AppAccountErasures_UserId",
                table: "AppAccountErasures",
                column: "UserId");

            migrationBuilder.CreateIndex(
                name: "IX_AppActivityErasures_WorkspaceId_CreationTime",
                table: "AppActivityErasures",
                columns: new[] { "WorkspaceId", "CreationTime" },
                descending: new[] { false, true });

            migrationBuilder.CreateIndex(
                name: "IX_AppActivityErasures_WorkspaceId_UserId_ClientActivityId",
                table: "AppActivityErasures",
                columns: new[] { "WorkspaceId", "UserId", "ClientActivityId" });
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "AppAccountErasures");

            migrationBuilder.DropTable(
                name: "AppActivityErasures");

            migrationBuilder.DropTable(
                name: "AppDataProtectionKeys");

            migrationBuilder.DropIndex(
                name: "IX_AppWorkspaces_IsDeleted_PurgedTime",
                table: "AppWorkspaces");

            migrationBuilder.DropIndex(
                name: "IX_AppWorkspaces_NormalizedName",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "DeleterId",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "DeletionTime",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "IsDeleted",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "PurgedTime",
                table: "AppWorkspaces");

            migrationBuilder.CreateIndex(
                name: "IX_AppWorkspaces_NormalizedName",
                table: "AppWorkspaces",
                column: "NormalizedName",
                unique: true);
        }
    }
}
