using System;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// REQ-DAT-01: Explicit workspace-scoped activity and time-summary retention policy specification.
/// Enforces bounded batching, positive retention duration, explicit workspace scoping, and exact-cutoff preservation.
/// </summary>
public class ActivityRetentionPolicy
{
    public const int DefaultBatchSize = 100;
    public const int MaxBatchSize = 1000;

    /// <summary>
    /// Explicit target workspace. Must not be empty.
    /// </summary>
    public Guid WorkspaceId { get; }

    /// <summary>
    /// Age-based retention duration. Activities strictly older than (Cutoff ?? (Now - RetentionPeriod)) are eligible for cleanup.
    /// Must be greater than TimeSpan.Zero.
    /// </summary>
    public TimeSpan RetentionPeriod { get; }

    /// <summary>
    /// Explicit offline ingestion horizon / admissible upload window (e.g. from ActivityIngestOptions.MaxBlockAge).
    /// Replay safety guard: an activity can only be deleted if its StartedAt is strictly older than (IngestionCutoff ?? (Now - IngestionHorizon)).
    /// Activities within the upload window must be preserved to guarantee idempotency and prevent recreating deleted blocks.
    /// Must be greater than TimeSpan.Zero unless IngestionCutoff is explicitly provided.
    /// </summary>
    public TimeSpan IngestionHorizon { get; }

    /// <summary>
    /// Maximum number of records to process in a single bounded batch (1..1000).
    /// </summary>
    public int BatchSize { get; }

    /// <summary>
    /// Explicit policy regarding archived workspaces.
    /// False (default): archived workspaces are skipped to prevent unexpected modification.
    /// True: allows cleaning expired activities from an archived workspace.
    /// </summary>
    public bool AllowArchivedWorkspace { get; }

    /// <summary>
    /// Optional deterministic cutoff timestamp. If provided, overrides (Now - RetentionPeriod).
    /// Exact-cutoff preservation: activities with StartedAt >= Cutoff survive; only StartedAt &lt; Cutoff are eligible.
    /// </summary>
    public DateTime? Cutoff { get; }

    /// <summary>
    /// Optional keyset pagination cursor for continuation past previously evaluated candidates.
    /// If null, query starts from the earliest expired candidate in (StartedAt, Id) order.
    /// </summary>
    public ActivityRetentionCursor? Cursor { get; }

    /// <summary>
    /// Optional deterministic cutoff timestamp for the ingestion horizon.
    /// Exact-boundary preservation: activities with StartedAt >= IngestionCutoff survive; only StartedAt &lt; IngestionCutoff are eligible.
    /// </summary>
    public DateTime? IngestionCutoff { get; }

    public ActivityRetentionPolicy(
        Guid workspaceId,
        TimeSpan retentionPeriod,
        TimeSpan ingestionHorizon,
        int batchSize = DefaultBatchSize,
        bool allowArchivedWorkspace = false,
        DateTime? cutoff = null,
        ActivityRetentionCursor? cursor = null,
        DateTime? ingestionCutoff = null)
    {
        WorkspaceId = workspaceId;
        RetentionPeriod = retentionPeriod;
        IngestionHorizon = ingestionHorizon;
        BatchSize = batchSize;
        AllowArchivedWorkspace = allowArchivedWorkspace;
        Cutoff = cutoff;
        Cursor = cursor;
        IngestionCutoff = ingestionCutoff;
    }

    /// <summary>
    /// Validates policy parameters before any database operations.
    /// Refuses empty workspace, zero/negative retention period, unknown/invalid ingestion horizon, or invalid batch sizes.
    /// </summary>
    public void Validate()
    {
        if (WorkspaceId == Guid.Empty)
        {
            throw new ArgumentException("Workspace ID must not be empty.", nameof(WorkspaceId));
        }

        if (RetentionPeriod <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(RetentionPeriod), "Retention period must be greater than zero.");
        }

        if (!IngestionCutoff.HasValue && IngestionHorizon <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(
                nameof(IngestionHorizon),
                "Ingestion horizon must be greater than zero to protect replay idempotency.");
        }

        if (BatchSize <= 0 || BatchSize > MaxBatchSize)
        {
            throw new ArgumentOutOfRangeException(nameof(BatchSize), $"Batch size must be between 1 and {MaxBatchSize}.");
        }
    }
}
