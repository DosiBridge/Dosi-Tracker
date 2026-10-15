using System;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp;
using Xunit;

namespace Dosi.Tracker.Reporting;

public class HourlyRateScheduleTests
{
    private static readonly Guid ProjectId = Guid.NewGuid();
    private static readonly Guid UserId = Guid.NewGuid();

    private static DateTime Utc(int month, int day, int hour = 0) => new(2026, month, day, hour, 0, 0, DateTimeKind.Utc);

    private static ProjectMemberRate Rate(decimal hourlyRate, DateTime effectiveFrom, DateTime? recordedAt = null)
    {
        var rate = new ProjectMemberRate(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, ProjectId, UserId, hourlyRate, effectiveFrom);
        ObjectHelper.TrySetProperty(rate, r => r.CreationTime, () => recordedAt ?? effectiveFrom);
        return rate;
    }

    [Fact]
    public void A_Schedule_Without_Rates_Should_Price_Everything_At_Zero()
    {
        HourlyRateSchedule.None.RateAt(Utc(7, 1)).ShouldBe(0m);
        HourlyRateSchedule.From(Array.Empty<ProjectMemberRate>()).RateAt(Utc(7, 1)).ShouldBe(0m);
    }

    [Fact]
    public void A_Flat_Schedule_Should_Always_Apply()
    {
        var schedule = HourlyRateSchedule.Flat(25m);

        schedule.RateAt(DateTime.MinValue).ShouldBe(25m);
        schedule.RateAt(Utc(7, 1)).ShouldBe(25m);
    }

    [Fact]
    public void RateAt_Should_Return_The_Latest_Rate_That_Took_Effect_By_The_Blocks_Start()
    {
        // Any order in, the history is sorted by when each rate took effect.
        var schedule = HourlyRateSchedule.From(new[]
        {
            Rate(50m, Utc(7, 15)),
            Rate(40m, Utc(7, 1)),
            Rate(45m, Utc(7, 10, 12))
        });

        schedule.RateAt(Utc(7, 1)).ShouldBe(40m);               // exactly when it took effect
        schedule.RateAt(Utc(7, 10, 11)).ShouldBe(40m);
        schedule.RateAt(Utc(7, 10, 12)).ShouldBe(45m);
        schedule.RateAt(Utc(7, 14, 23)).ShouldBe(45m);
        schedule.RateAt(Utc(9, 1)).ShouldBe(50m);
        schedule.RateAt(new DateTime(2026, 7, 12, 0, 0, 0, DateTimeKind.Unspecified)).ShouldBe(45m); // read as UTC
    }

    [Fact]
    public void RateAt_Should_Use_The_First_Rate_For_A_Block_That_Started_Before_The_History()
    {
        var schedule = HourlyRateSchedule.From(new[] { Rate(40m, Utc(7, 1)), Rate(50m, Utc(8, 1)) });

        schedule.RateAt(Utc(6, 30, 23)).ShouldBe(40m);
    }

    [Fact]
    public void RateAt_Should_Prefer_The_Rate_Recorded_Last_When_Two_Take_Effect_At_The_Same_Moment()
    {
        var schedule = HourlyRateSchedule.From(new[]
        {
            Rate(30m, Utc(7, 1), recordedAt: Utc(7, 1, 9)),
            Rate(35m, Utc(7, 1), recordedAt: Utc(7, 1, 10)),
            Rate(20m, Utc(6, 1))
        });

        schedule.RateAt(Utc(7, 1)).ShouldBe(35m);
        schedule.RateAt(Utc(7, 2)).ShouldBe(35m);
        schedule.RateAt(Utc(6, 15)).ShouldBe(20m);
    }
}
