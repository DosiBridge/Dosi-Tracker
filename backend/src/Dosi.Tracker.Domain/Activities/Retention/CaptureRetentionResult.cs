using System;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Result of an explicit capture retention cleanup run.
/// </summary>
public class CaptureRetentionResult
{
    public Guid WorkspaceId { get; init; }
    public DateTime Cutoff { get; init; }
    public int CandidateCount { get; init; }
    public int DeletedCount { get; init; }
    public int FailedBlobCount { get; init; }
    public int MissingBlobCount { get; init; }
    public int CorruptKeyCount { get; init; }
    public bool HasMore { get; init; }
    public bool Skipped { get; init; }
    public string? SkipReason { get; init; }
}
