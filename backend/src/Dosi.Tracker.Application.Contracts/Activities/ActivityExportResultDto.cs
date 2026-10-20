using System;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Result of an activity CSV export, including file bytes, metadata, and record counts.
/// </summary>
public class ActivityExportResultDto
{
    public byte[] Bytes { get; set; } = Array.Empty<byte>();
    public string ContentType { get; set; } = "text/csv; charset=utf-8";
    public string FileName { get; set; } = "activities.csv";
    public int TotalCount { get; set; }
    public int ExportedCount { get; set; }
}
