using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Activities;

/// <summary>
/// The minute buckets of a block, as both desktop agents cut them (shared contract §1, a port of
/// <c>clients/windows/src/tracking/input.rs</c>): the block length rounded to the nearest minute, at least one;
/// bucket i covers seconds [60·i, 60·(i+1)) from the start and the last bucket absorbs the remainder, so it
/// lasts 30..89 s (or the whole block when that is shorter than 90 s).
/// </summary>
internal static class ActivityMinuteBuckets
{
    /// <summary>The block length in whole seconds (rounded half away from zero).</summary>
    public static int SpanSeconds(TimeSpan blockLength) =>
        blockLength <= TimeSpan.Zero ? 0 : (int)Math.Round(blockLength.TotalSeconds, MidpointRounding.AwayFromZero);

    /// <summary>Number of buckets of a block lasting <paramref name="spanSeconds"/>; zero for an empty block.</summary>
    public static int Count(int spanSeconds) => spanSeconds <= 0 ? 0 : Math.Max(1, (spanSeconds + 30) / 60);

    /// <summary>Length in seconds of bucket <paramref name="minute"/>; zero for an index outside the block.</summary>
    public static int LengthSeconds(int minute, int spanSeconds)
    {
        var count = Count(spanSeconds);
        if (minute < 0 || minute >= count)
        {
            return 0;
        }

        return minute < count - 1 ? 60 : spanSeconds - 60 * (count - 1);
    }

    /// <summary>Idle seconds of a block derived from its timeline: the summed length of the buckets reported as
    /// not active, never more than the block itself. A bucket listed twice counts once, as its first entry.</summary>
    public static int IdleSeconds(IEnumerable<ActivityMinuteDto> timeline, TimeSpan blockLength)
    {
        var span = SpanSeconds(blockLength);
        var seen = new HashSet<int>();
        var idle = 0;
        foreach (var minute in timeline)
        {
            if (minute == null || !seen.Add(minute.Minute))
            {
                continue;
            }

            if (!minute.Active)
            {
                idle += LengthSeconds(minute.Minute, span);
            }
        }

        return Math.Min(idle, span);
    }
}
