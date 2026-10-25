namespace Dosi.Tracker.Erasure;

/// <summary>
/// The erasure sweep (<see cref="DataErasureWorker"/>), configuration section <c>DataErasure</c>: it purges the data of
/// deleted workspaces once their grace period is over, and scrubs the tracked data of deleted accounts.
/// </summary>
public class DataErasureOptions
{
    public const string SectionName = "DataErasure";

    /// <summary>Whether the sweep runs. Off only for an operator who erases by other means: deleted workspaces and
    /// accounts then keep their data.</summary>
    public bool IsEnabled { get; set; } = true;

    /// <summary>Minutes between sweeps (at least 1). The first sweep runs at start-up.</summary>
    public int PeriodMinutes { get; set; } = 60;

    /// <summary>Days a deleted workspace's data is kept before the sweep purges it, so an operator can still recover a
    /// workspace deleted by mistake (0: the next sweep purges it).</summary>
    public int WorkspaceGraceDays { get; set; } = 7;

    /// <summary>Captures or blocks handled per database round trip.</summary>
    public int BatchSize { get; set; } = 200;
}
