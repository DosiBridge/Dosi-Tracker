namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Summary result of an automated capture retention sweep across workspaces.
/// </summary>
public class CaptureRetentionSweepResult
{
    public bool Skipped { get; set; }
    public string? SkipReason { get; set; }
    public int WorkspacesSwept { get; set; }
    public int TotalDeleted { get; set; }
    public int TotalFailedBlobs { get; set; }
    public int TotalMissingBlobs { get; set; }
    public int TotalCorruptKeys { get; set; }
    public int TotalBatchesProcessed { get; set; }
    public int Errors { get; set; }
}
