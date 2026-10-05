using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.Projects;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// One member's hourly-rate history on one project (<see cref="ProjectMemberRate"/> rows), answering "which rate
/// was in effect when this block started?". Payroll prices every block with it, so changing a rate (or removing
/// the member) never re-prices time tracked before the change.
/// </summary>
public sealed class HourlyRateSchedule
{
    /// <summary>A schedule without any rate: everything is priced at 0.</summary>
    public static readonly HourlyRateSchedule None = new(Array.Empty<DateTime>(), Array.Empty<decimal>());

    private readonly DateTime[] _effectiveFrom;
    private readonly decimal[] _rates;

    private HourlyRateSchedule(DateTime[] effectiveFrom, decimal[] rates)
    {
        _effectiveFrom = effectiveFrom;
        _rates = rates;
    }

    /// <summary>The schedule of a rate history (any order). Entries that take effect at the same moment resolve to
    /// the one recorded last.</summary>
    public static HourlyRateSchedule From(IEnumerable<ProjectMemberRate> history)
    {
        var entries = history
            .OrderBy(r => r.EffectiveFrom)
            .ThenBy(r => r.CreationTime)
            .ThenBy(r => r.Id)
            .ToList();

        return entries.Count == 0
            ? None
            : new HourlyRateSchedule(
                entries.Select(r => ReportCalendar.AsUtc(r.EffectiveFrom)).ToArray(),
                entries.Select(r => r.HourlyRate).ToArray());
    }

    /// <summary>A single rate that has always applied (a membership without recorded history).</summary>
    public static HourlyRateSchedule Flat(decimal hourlyRate)
    {
        return new HourlyRateSchedule(new[] { DateTime.MinValue }, new[] { hourlyRate });
    }

    /// <summary>
    /// The rate in effect at <paramref name="startedAtUtc"/>: the latest entry whose EffectiveFrom is not after it.
    /// A block that started before the first entry (a clock that ran behind, or time uploaded from before the member
    /// was added) is priced at the first rate the member was given.
    /// </summary>
    public decimal RateAt(DateTime startedAtUtc)
    {
        if (_rates.Length == 0)
        {
            return 0m;
        }

        var start = ReportCalendar.AsUtc(startedAtUtc);
        var index = Array.BinarySearch(_effectiveFrom, start);
        if (index < 0)
        {
            // No exact match: ~index is the first entry that starts after the block, so the one before applies.
            index = ~index - 1;
        }
        else
        {
            // Exact match: several entries may share this moment, and the last one recorded wins.
            while (index + 1 < _effectiveFrom.Length && _effectiveFrom[index + 1] == start)
            {
                index++;
            }
        }

        return _rates[Math.Max(index, 0)];
    }
}
