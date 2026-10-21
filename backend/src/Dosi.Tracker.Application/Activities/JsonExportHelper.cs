using System;
using System.Collections.Generic;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Formats activity records into a versioned JSON summary document (REQ-DAT-02).
/// Enforces strict field allowlist: excludes raw window titles, process lists, screenshots,
/// blob URLs, pay rates, credentials, and preserves known unavailable metrics as null.
/// </summary>
public static class JsonExportHelper
{
    public const string SchemaVersion = "1.0";

    public static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        WriteIndented = false
    };

    /// <summary>
    /// Builds full UTF-8 JSON byte content for the given activities and resolved project titles.
    /// </summary>
    public static byte[] BuildActivitiesJson(
        Guid workspaceId,
        DateTime exportedAtUtc,
        IReadOnlyList<Activity> activities,
        IReadOnlyDictionary<Guid, string> projectTitles)
    {
        var items = new List<ActivityJsonExportItemDto>(activities.Count);

        foreach (var a in activities)
        {
            var projectTitle = projectTitles.TryGetValue(a.ProjectId, out var title) ? title : string.Empty;
            var startUtc = a.StartedAt.ToUniversalTime();
            var endUtc = a.EndedAt.ToUniversalTime();
            var duration = endUtc - startUtc;

            var durationMinutes = Math.Round(duration.TotalMinutes, 2);
            var durationSeconds = Math.Max(0, (int)Math.Round(duration.TotalSeconds));

            items.Add(new ActivityJsonExportItemDto
            {
                ActivityId = a.Id,
                UserId = a.UserId,
                ProjectId = a.ProjectId,
                ProjectTitle = projectTitle,
                StartedAtUtc = startUtc,
                EndedAtUtc = endUtc,
                DurationMinutes = durationMinutes,
                DurationSeconds = durationSeconds,
                Description = a.Description,
                Productivity = a.Productivity,
                IdleSeconds = a.IdleSeconds,
                MouseClicks = a.MouseClicks,
                KeyboardHits = a.KeyboardHits
            });
        }

        var document = new ActivityJsonExportDocumentDto
        {
            Version = SchemaVersion,
            WorkspaceId = workspaceId,
            ExportedAtUtc = exportedAtUtc,
            TotalCount = items.Count,
            Activities = items
        };

        return JsonSerializer.SerializeToUtf8Bytes(document, JsonOptions);
    }
}
