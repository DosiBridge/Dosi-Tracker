using System;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Filters for exporting workspace activities to CSV.
/// Workload and date bounds are strictly enforced by the server:
/// malformed ranges (To &lt;= From) or spans exceeding <see cref="MaxRangeDays"/> days are rejected.
/// The server rejects requested limits or candidate workloads exceeding <see cref="MaxAllowedResultCount"/>.
/// </summary>
public class ExportActivitiesInput
{
    public const int DefaultMaxResultCount = 1000;
    public const int MaxAllowedResultCount = 10000;
    public const int MaxRangeDays = 366;

    /// <summary>Starting UTC timestamp (inclusive).</summary>
    public DateTime? From { get; set; }

    /// <summary>Ending UTC timestamp (exclusive).</summary>
    public DateTime? To { get; set; }

    /// <summary>Optional project filter within the authorized workspace.</summary>
    public Guid? ProjectId { get; set; }

    /// <summary>
    /// Optional user filter: only honored for callers with <c>Tracker.Activities.ViewAll</c> permission (Owner/Manager).
    /// Regular members requesting another user's activity are rejected with 403 Forbidden.
    /// </summary>
    public Guid? UserId { get; set; }

    /// <summary>
    /// Explicit requested maximum result count. Must be between 1 and <see cref="MaxAllowedResultCount"/>.
    /// If candidate records exceed this limit, the request is rejected with 400 Bad Request to prevent silent partial truncation.
    /// </summary>
    public int? MaxResultCount { get; set; }
}
