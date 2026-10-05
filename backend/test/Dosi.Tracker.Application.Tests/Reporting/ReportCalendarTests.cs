using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Reporting;

public class ReportCalendarTests
{
    private static readonly TimeZoneInfo NewYork = ReportCalendar.ResolveZone("America/New_York");

    private static DateTime Utc(int year, int month, int day, int hour = 0, int minute = 0) =>
        new(year, month, day, hour, minute, 0, DateTimeKind.Utc);

    /// <summary>
    /// A UTC-4 zone whose clocks jump from 00:00 to 01:00 on 6 September (so that day has no midnight) and fall back
    /// from 01:00 to 00:00 on 5 April (so that day's midnight happens twice) — transitions at midnight, as some real
    /// zones have, built in code so the test does not depend on the machine's time zone data.
    /// </summary>
    private static TimeZoneInfo MidnightTransitionZone()
    {
        var rule = TimeZoneInfo.AdjustmentRule.CreateAdjustmentRule(
            new DateTime(2000, 1, 1),
            new DateTime(2099, 12, 31),
            TimeSpan.FromHours(1),
            TimeZoneInfo.TransitionTime.CreateFixedDateRule(new DateTime(1, 1, 1, 0, 0, 0), 9, 6),
            TimeZoneInfo.TransitionTime.CreateFixedDateRule(new DateTime(1, 1, 1, 1, 0, 0), 4, 5));

        return TimeZoneInfo.CreateCustomTimeZone(
            "Test/Midnight", TimeSpan.FromHours(-4), "Midnight transitions", "Standard", "Daylight", new[] { rule });
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("  ")]
    [InlineData("Mars/Olympus_Mons")]
    [InlineData("America/New_York/../../etc")]
    public void ResolveZone_Should_Fall_Back_To_Utc_For_A_Missing_Or_Unknown_Id(string? id)
    {
        ReportCalendar.ResolveZone(id).ShouldBe(TimeZoneInfo.Utc);
        ReportCalendar.ResolveZone(new string('x', 65)).ShouldBe(TimeZoneInfo.Utc);
    }

    [Fact]
    public void ResolveZone_Should_Find_Iana_Zones()
    {
        ReportCalendar.ResolveZone(" Asia/Kathmandu ").BaseUtcOffset.ShouldBe(new TimeSpan(5, 45, 0));
        NewYork.GetUtcOffset(Utc(2026, 7, 1)).ShouldBe(TimeSpan.FromHours(-4));
        NewYork.GetUtcOffset(Utc(2026, 1, 1)).ShouldBe(TimeSpan.FromHours(-5));
    }

    [Theory]
    [InlineData("member", true)]
    [InlineData(" Member ", true)]
    [InlineData("zone", false)]
    [InlineData("members", false)]
    [InlineData(null, false)]
    public void IsMemberBasis_Should_Only_Accept_Member(string? dayBasis, bool expected)
    {
        ReportCalendar.IsMemberBasis(dayBasis).ShouldBe(expected);
    }

    [Fact]
    public void DaysOverlapping_Should_List_Every_Local_Day_The_Range_Touches()
    {
        var utc = new ReportCalendar(TimeZoneInfo.Utc, memberBasis: false);
        utc.DaysOverlapping(Utc(2026, 7, 20), Utc(2026, 7, 23))
            .ShouldBe(new[] { new DateTime(2026, 7, 20), new DateTime(2026, 7, 21), new DateTime(2026, 7, 22) });

        // A rolling range touches partial days at both ends.
        utc.DaysOverlapping(Utc(2026, 7, 20, 15), Utc(2026, 7, 21, 9))
            .ShouldBe(new[] { new DateTime(2026, 7, 20), new DateTime(2026, 7, 21) });

        // Local days, across the 23-hour spring-forward Sunday.
        var newYork = new ReportCalendar(NewYork, memberBasis: false);
        var days = newYork.DaysOverlapping(Utc(2026, 3, 7, 5), Utc(2026, 3, 10, 4));
        days.ShouldBe(new[] { new DateTime(2026, 3, 7), new DateTime(2026, 3, 8), new DateTime(2026, 3, 9) });
        days.ShouldAllBe(d => d.Kind == DateTimeKind.Unspecified);
    }

    [Fact]
    public void StartOfDayUtc_Should_Be_The_Local_Midnight()
    {
        var newYork = new ReportCalendar(NewYork, memberBasis: false);

        newYork.StartOfDayUtc(new DateTime(2026, 7, 20)).ShouldBe(Utc(2026, 7, 20, 4));
        newYork.StartOfDayUtc(new DateTime(2026, 3, 8)).ShouldBe(Utc(2026, 3, 8, 5));   // still EST at midnight
        newYork.StartOfDayUtc(new DateTime(2026, 3, 9)).ShouldBe(Utc(2026, 3, 9, 4));   // EDT from the 8th
        newYork.StartOfDayUtc(new DateTime(2026, 3, 9)).Kind.ShouldBe(DateTimeKind.Utc);
        new ReportCalendar(ReportCalendar.ResolveZone("Asia/Kathmandu"), false)
            .StartOfDayUtc(new DateTime(2026, 7, 20)).ShouldBe(Utc(2026, 7, 19, 18, 15));
    }

    [Fact]
    public void StartOfDayUtc_Should_Handle_Clocks_That_Skip_Or_Repeat_Midnight()
    {
        var calendar = new ReportCalendar(MidnightTransitionZone(), memberBasis: false);

        // 6 September has no 00:00: the day starts at the jump, 04:00Z (01:00 local).
        var skipped = calendar.StartOfDayUtc(new DateTime(2026, 9, 6));
        skipped.ShouldBe(Utc(2026, 9, 6, 4));
        calendar.ZoneClock.ToLocal(skipped).ShouldBe(new DateTime(2026, 9, 6, 1, 0, 0));
        calendar.ZoneClock.ToLocal(skipped.AddTicks(-1)).Date.ShouldBe(new DateTime(2026, 9, 5));

        // 5 April's 00:00 happens twice: the day starts at the first one, 03:00Z (still on daylight time).
        var repeated = calendar.StartOfDayUtc(new DateTime(2026, 4, 5));
        repeated.ShouldBe(Utc(2026, 4, 5, 3));
        calendar.ZoneClock.ToLocal(repeated.AddTicks(-1)).Date.ShouldBe(new DateTime(2026, 4, 4));
    }

    [Fact]
    public void ClockFor_Should_Use_The_Requested_Zone_With_The_Zone_Basis()
    {
        var calendar = new ReportCalendar(NewYork, memberBasis: false);

        calendar.ClockFor("Asia/Dhaka", 360).ToLocal(Utc(2026, 7, 21, 2)).ShouldBe(new DateTime(2026, 7, 20, 22, 0, 0));
    }

    [Fact]
    public void ClockFor_Should_Prefer_The_Devices_Offset_Then_Its_Zone_Then_The_Requested_Zone_With_The_Member_Basis()
    {
        var calendar = new ReportCalendar(NewYork, memberBasis: true);
        var instant = Utc(2026, 7, 21, 2);

        calendar.ClockFor("Asia/Tokyo", 360).ToLocal(instant).ShouldBe(new DateTime(2026, 7, 21, 8, 0, 0));   // offset wins
        calendar.ClockFor("Asia/Tokyo", null).ToLocal(instant).ShouldBe(new DateTime(2026, 7, 21, 11, 0, 0)); // device zone
        calendar.ClockFor("Asia/Tokyo", 900).ToLocal(instant).ShouldBe(new DateTime(2026, 7, 21, 11, 0, 0));  // offset out of range
        calendar.ClockFor("Nowhere/Land", null).ToLocal(instant).ShouldBe(new DateTime(2026, 7, 20, 22, 0, 0)); // requested zone
        calendar.ClockFor(null, -840).ToLocal(instant).ShouldBe(new DateTime(2026, 7, 20, 12, 0, 0));
    }

    [Fact]
    public void LocalClock_Should_Return_Wall_Clock_Times_And_Read_Unspecified_Input_As_Utc()
    {
        var fixedClock = LocalClock.Fixed(TimeSpan.FromMinutes(345));
        var local = fixedClock.ToLocal(new DateTime(2026, 7, 20, 18, 15, 0, DateTimeKind.Unspecified));

        local.ShouldBe(new DateTime(2026, 7, 21));
        local.Kind.ShouldBe(DateTimeKind.Unspecified);
        LocalClock.Of(TimeZoneInfo.Utc).ToLocal(Utc(2026, 7, 20, 9)).Kind.ShouldBe(DateTimeKind.Unspecified);

        ReportCalendar.AsUtc(new DateTime(2026, 7, 20, 9, 0, 0, DateTimeKind.Unspecified)).Kind.ShouldBe(DateTimeKind.Utc);
        ReportCalendar.AsUtc(Utc(2026, 7, 20, 9)).ShouldBe(Utc(2026, 7, 20, 9));
        var localTime = new DateTime(2026, 7, 20, 9, 0, 0, DateTimeKind.Local);
        ReportCalendar.AsUtc(localTime).ShouldBe(localTime.ToUniversalTime());
    }

    [Fact]
    public void For_Should_Read_The_Report_Input()
    {
        var calendar = ReportCalendar.For(new GetReportSummaryInput { TimeZone = "Asia/Kathmandu", DayBasis = "member" });

        calendar.Zone.BaseUtcOffset.ShouldBe(new TimeSpan(5, 45, 0));
        calendar.MemberBasis.ShouldBeTrue();
        ReportCalendar.For(new GetReportSummaryInput()).Zone.ShouldBe(TimeZoneInfo.Utc);
        ReportCalendar.For(new GetReportSummaryInput()).MemberBasis.ShouldBeFalse();
    }
}
