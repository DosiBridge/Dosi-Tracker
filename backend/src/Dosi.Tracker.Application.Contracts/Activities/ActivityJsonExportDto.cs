using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Root document for workspace activity JSON summary export (REQ-DAT-02).
/// Versioned minimal summary schema with strict field allowlist.
/// </summary>
public class ActivityJsonExportDocumentDto
{
    /// <summary>"1.1": MouseClicks and KeyboardHits became nullable (null = not measured).</summary>
    public string Version { get; set; } = "1.1";
    public Guid WorkspaceId { get; set; }
    public DateTime ExportedAtUtc { get; set; }
    public int TotalCount { get; set; }
    public List<ActivityJsonExportItemDto> Activities { get; set; } = new();
}

/// <summary>
/// Individual activity summary record within JSON export.
/// Excludes sensitive captures, window titles, process lists, blob URLs, pay/credentials.
/// Unavailable metrics are preserved as null.
/// </summary>
public class ActivityJsonExportItemDto
{
    public Guid ActivityId { get; set; }
    public Guid UserId { get; set; }
    public Guid ProjectId { get; set; }
    public string ProjectTitle { get; set; } = string.Empty;
    public DateTime StartedAtUtc { get; set; }
    public DateTime EndedAtUtc { get; set; }
    public double DurationMinutes { get; set; }
    public int DurationSeconds { get; set; }
    public string? Description { get; set; }
    public int? Productivity { get; set; }
    public int? IdleSeconds { get; set; }
    public int? MouseClicks { get; set; }
    public int? KeyboardHits { get; set; }
}
