using System;
using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace Dosi.Tracker.Migrations
{
    /// <inheritdoc />
    public partial class AddWorkspaceArchivalFields : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.AddColumn<Guid>(
                name: "ArchivedBy",
                table: "AppWorkspaces",
                type: "uuid",
                nullable: true);

            migrationBuilder.AddColumn<DateTime>(
                name: "ArchivedTime",
                table: "AppWorkspaces",
                type: "timestamp without time zone",
                nullable: true);

            migrationBuilder.AddColumn<bool>(
                name: "IsArchived",
                table: "AppWorkspaces",
                type: "boolean",
                nullable: false,
                defaultValue: false);

            migrationBuilder.CreateIndex(
                name: "IX_AppWorkspaces_IsArchived",
                table: "AppWorkspaces",
                column: "IsArchived");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropIndex(
                name: "IX_AppWorkspaces_IsArchived",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "ArchivedBy",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "ArchivedTime",
                table: "AppWorkspaces");

            migrationBuilder.DropColumn(
                name: "IsArchived",
                table: "AppWorkspaces");
        }
    }
}
