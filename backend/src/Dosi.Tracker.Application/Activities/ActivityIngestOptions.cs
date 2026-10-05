using System;

namespace Dosi.Tracker.Activities;

/// <summary>Tunable limits of the block upload (<c>POST /api/app/activity</c>, shared contract §1).</summary>
public class ActivityIngestOptions
{
    /// <summary>The contract's upload window: blocks that started longer ago are refused.</summary>
    public static readonly TimeSpan DefaultMaxBlockAge = TimeSpan.FromDays(45);

    /// <summary>
    /// How long after its start a block may still be uploaded; older blocks are refused with
    /// <c>Tracker:ActivityTooOld</c> (422). It bounds how far back time can be added to closed periods (payroll),
    /// and should cover the agents' offline queue retention. Default: 45 days.
    /// </summary>
    public TimeSpan MaxBlockAge { get; set; } = DefaultMaxBlockAge;
}
