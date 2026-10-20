using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// REQ-DAT-01: Explicit workspace-scoped capture retention policy specification.
/// Enforces bounded batching, positive retention duration, and explicit workspace scoping.
/// </summary>
public class CaptureRetentionPolicy
{
    public const int DefaultBatchSize = 100;
    public const int MaxBatchSize = 1000;

    /// <summary>
    /// Explicit target workspace. Must not be empty.
    /// </summary>
    public Guid WorkspaceId { get; }

    /// <summary>
    /// Age-based retention duration. Captures older than (Now - RetentionPeriod) are eligible for cleanup.
    /// Must be greater than TimeSpan.Zero.
    /// </summary>
    public TimeSpan RetentionPeriod { get; }

    /// <summary>
    /// Maximum number of records to process in a single bounded batch (1..1000).
    /// </summary>
    public int BatchSize { get; }

    /// <summary>
    /// Optional capture kinds to clean up (e.g., "screen", "thumb", "webcam").
    /// Null or empty indicates all capture kinds.
    /// </summary>
    public IReadOnlyList<string>? Kinds { get; }

    /// <summary>
    /// Explicit policy regarding archived workspaces.
    /// False (default): archived workspaces are skipped to prevent unexpected modification.
    /// True: allows cleaning expired captures from an archived workspace.
    /// </summary>
    public bool AllowArchivedWorkspace { get; }

    /// <summary>
    /// Optional deterministic cutoff timestamp. If provided, overrides (Now - RetentionPeriod).
    /// </summary>
    public DateTime? Cutoff { get; }

    public CaptureRetentionPolicy(
        Guid workspaceId,
        TimeSpan retentionPeriod,
        int batchSize = DefaultBatchSize,
        IReadOnlyList<string>? kinds = null,
        bool allowArchivedWorkspace = false,
        DateTime? cutoff = null)
    {
        WorkspaceId = workspaceId;
        RetentionPeriod = retentionPeriod;
        BatchSize = batchSize;
        Kinds = kinds;
        AllowArchivedWorkspace = allowArchivedWorkspace;
        Cutoff = cutoff;
    }

    /// <summary>
    /// Validates policy parameters before any database or storage operations.
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

        if (BatchSize <= 0 || BatchSize > MaxBatchSize)
        {
            throw new ArgumentOutOfRangeException(nameof(BatchSize), $"Batch size must be between 1 and {MaxBatchSize}.");
        }
    }
}
