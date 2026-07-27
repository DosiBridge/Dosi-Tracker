using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AddCaptureSettingsAndScreenshotKinds : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppScreenshots_ActivityId",
                table: "AppScreenshots");

            migrationBuilder.AddColumn<string>(
                name: "Kind",
                table: "AppScreenshots",
                type: "character varying(16)",
                maxLength: 16,
                nullable: false,
                defaultValue: "screen");

            migrationBuilder.AddColumn<bool>(
                name: "AllowActiveWindow",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: true);

            migrationBuilder.AddColumn<bool>(
                name: "AllowKeyboard",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: true);

            migrationBuilder.AddColumn<bool>(
                name: "AllowMouse",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: true);

            migrationBuilder.AddColumn<bool>(
                name: "AllowRunningPrograms",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: true);

            migrationBuilder.AddColumn<bool>(
                name: "AllowScreenshot",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: true);

            migrationBuilder.AddColumn<bool>(
                name: "AllowWebcam",
                table: "AppProjects",
                type: "boolean",
                nullable: false,
                defaultValue: false);

            migrationBuilder.CreateIndex(
                name: "IX_AppScreenshots_ActivityId",
                table: "AppScreenshots",
                column: "ActivityId");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppScreenshots_ActivityId",
                table: "AppScreenshots");

            migrationBuilder.DropColumn(
                name: "Kind",
                table: "AppScreenshots");

            migrationBuilder.DropColumn(
                name: "AllowActiveWindow",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "AllowKeyboard",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "AllowMouse",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "AllowRunningPrograms",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "AllowScreenshot",
                table: "AppProjects");

            migrationBuilder.DropColumn(
                name: "AllowWebcam",
                table: "AppProjects");

            migrationBuilder.CreateIndex(
                name: "IX_AppScreenshots_ActivityId",
                table: "AppScreenshots",
                column: "ActivityId",
                unique: true);
        }
    }
}
