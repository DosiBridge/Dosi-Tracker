namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Platform configuration of the activity retention worker (section <c>ActivityRetention</c>). Like capture retention,
/// the worker deletes time blocks only of workspaces whose owner opted in to a retention window
/// (<c>PUT /api/app/workspaces/{id}/activity-retention</c>); <see cref="IsEnabled"/> = false stops every sweep.
/// </summary>
public class ActivityRetentionOptions
{
    /// <summary>Configuration section the options are bound from.</summary>
    public const string SectionName = "ActivityRetention";

    /// <summary>Master switch for the activity retention worker. Defaults to true: owners' policies are enforced.</summary>
    public bool IsEnabled { get; set; } = true;

    /// <summary>Hours between sweeps; the first runs at start-up. Defaults to 24 hours.</summary>
    public int PeriodHours { get; set; } = 24;

    /// <summary>Blocks evaluated per batch (1..<see cref="ActivityRetentionPolicy.MaxBatchSize"/>).</summary>
    public int BatchSize { get; set; } = ActivityRetentionPolicy.DefaultBatchSize;

    /// <summary>Batches per workspace and sweep, which bounds one sweep's work; the next sweep continues.</summary>
    public int MaxPagesPerWorkspace { get; set; } = 10;
}
