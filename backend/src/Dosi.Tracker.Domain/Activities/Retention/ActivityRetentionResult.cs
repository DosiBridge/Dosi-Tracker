using System;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Result summary of an activity retention cleanup batch (REQ-DAT-01).
/// </summary>
public class ActivityRetentionResult
{
    public Guid WorkspaceId { get; set; }
    public DateTime Cutoff { get; set; }
    public DateTime IngestionCutoff { get; set; }
    public int CandidateCount { get; set; }
    public int DeletedCount { get; set; }
    public int PreservedReferencedCount { get; set; }
    public int PreservedReplaySafetyCount { get; set; }
    public bool HasMore { get; set; }
    public ActivityRetentionCursor? NextCursor { get; set; }
    public ActivityRetentionCursor? LastProcessedCursor { get; set; }
    public bool Skipped { get; set; }
    public string? SkipReason { get; set; }
}
