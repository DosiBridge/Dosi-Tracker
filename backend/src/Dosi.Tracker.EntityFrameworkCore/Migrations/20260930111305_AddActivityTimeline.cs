using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AddActivityTimeline : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<string>(
                name: "TimelineJson",
                table: "AppActivities",
                type: "text",
                nullable: false,
                defaultValue: "[]"); // Existing blocks have no timeline: an empty JSON array, never "" or NULL.
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropColumn(
                name: "TimelineJson",
                table: "AppActivities");
        }
    }
}
