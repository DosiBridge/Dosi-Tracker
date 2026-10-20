namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Operational configuration for capture retention worker and policies.
/// Default is strictly disabled (IsEnabled = false) to ensure no accidental or unapproved deletion.
/// </summary>
public class CaptureRetentionOptions
{
    /// <summary>
    /// Master switch for automated capture retention worker. Defaults to false.
    /// </summary>
    public bool IsEnabled { get; set; } = false;

    /// <summary>
    /// Periodic worker interval in hours. Defaults to 24 hours.
    /// </summary>
    public int PeriodHours { get; set; } = 24;

    /// <summary>
    /// Default retention window in days when automated worker runs. Defaults to 30 days.
    /// </summary>
    public int DefaultRetentionDays { get; set; } = 30;

    /// <summary>
    /// Maximum number of items processed per batch in automated sweeps.
    /// </summary>
    public int BatchSize { get; set; } = CaptureRetentionPolicy.DefaultBatchSize;

    /// <summary>
    /// Maximum number of batches (pages) to process per workspace within a single periodic sweep.
    /// Bounds the execution time and provider operations per sweep while guaranteeing bounded forward progress.
    /// Defaults to 5.
    /// </summary>
    public int MaxPagesPerWorkspace { get; set; } = 5;
}
