using System;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// Running totals of a group of blocks (a member, a project, a day, …), fed one block at a time so a report never
/// holds the range's rows in memory. Minutes are real block durations; idle minutes come only from blocks that
/// measured them; activity % is the duration-weighted mean of the blocks that measured it.
/// </summary>
internal sealed class BlockTotals
{
    private double _measuredMinutes;
    private double _weightedProductivity;
    private decimal _pricedMinutes;
    private decimal _amount;

    public int Count { get; private set; }
    public double Minutes { get; private set; }
    public double IdleMinutes { get; private set; }
    public long MouseClicks { get; private set; }
    public long KeyboardHits { get; private set; }

    public void Add(double minutes, int? productivity, double idleMinutes, long mouseClicks = 0, long keyboardHits = 0)
    {
        Count++;
        Minutes += minutes;
        IdleMinutes += idleMinutes;
        MouseClicks += mouseClicks;
        KeyboardHits += keyboardHits;

        // Unmeasured blocks (null) are left out of both sums, so they never drag the average towards 0%.
        if (productivity.HasValue)
        {
            _measuredMinutes += minutes;
            _weightedProductivity += minutes * productivity.Value;
        }
    }

    /// <summary>Prices <paramref name="minutes"/> at <paramref name="hourlyRate"/> (money stays in decimal).</summary>
    public void AddPay(double minutes, decimal hourlyRate)
    {
        var priced = (decimal)minutes;
        _pricedMinutes += priced;
        _amount += priced * hourlyRate / 60m;
    }

    /// <summary>Duration-weighted mean, so a 10-minute focused block outweighs a 1-minute idle one; null when no
    /// block of the group measured activity.</summary>
    public double? AverageProductivity =>
        _measuredMinutes <= 0 ? null : Math.Round(_weightedProductivity / _measuredMinutes, 2);

    /// <summary>The priced amount, rounded to cents (midpoints away from zero, as invoices round).</summary>
    public decimal Amount => Math.Round(_amount, 2, MidpointRounding.AwayFromZero);

    /// <summary>The rates in effect, weighted by the minutes priced at each; 0 when nothing was priced.</summary>
    public decimal HourlyRate =>
        _pricedMinutes <= 0 ? 0m : Math.Round(_amount * 60m / _pricedMinutes, 2, MidpointRounding.AwayFromZero);

    public double RoundedMinutes => Math.Round(Minutes, 2);

    public double RoundedIdleMinutes => Math.Round(IdleMinutes, 2);
}
