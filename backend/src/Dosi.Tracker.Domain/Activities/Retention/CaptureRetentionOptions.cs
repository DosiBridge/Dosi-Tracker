namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Platform configuration of the capture retention worker (section <c>CaptureRetention</c>, bound whatever the blob
/// provider). The worker only ever deletes captures of workspaces whose owner opted in to a retention window
/// (<c>PUT /api/app/workspaces/{id}/capture-retention</c>); a workspace without a policy keeps its captures. So the worker
/// runs by default, and <see cref="IsEnabled"/> = false is the operator's switch to stop every sweep (owners then see
/// their policy reported as not enforced).
/// </summary>
public class CaptureRetentionOptions
{
    /// <summary>Configuration section the options are bound from.</summary>
    public const string SectionName = "CaptureRetention";

    /// <summary>
    /// Master switch for the capture retention worker. Defaults to true: owners' policies are enforced.
    /// </summary>
    public bool IsEnabled { get; set; } = true;

    /// <summary>
    /// Hours between sweeps; the first runs at start-up (under the sweep lock), so restarts never postpone it.
    /// Defaults to 24 hours.
    /// </summary>
    public int PeriodHours { get; set; } = 24;

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
