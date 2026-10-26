using System;
using Dosi.Tracker.Reporting;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary><see cref="DisplayFormat"/>: the one way values are written into a bundle.</summary>
public class DisplayFormatTests
{
    [Theory]
    [InlineData(412, "6 h 52 min")]
    [InlineData(45, "45 min")]
    [InlineData(120, "2 h")]
    [InlineData(59.6, "1 h")]
    [InlineData(0.3, "less than 1 min")]
    [InlineData(0, "0 min")]
    [InlineData(-5, "0 min")]
    [InlineData(double.NaN, "0 min")]
    public void Duration_Should_Read_In_Hours_And_Minutes(double minutes, string expected)
    {
        DisplayFormat.Duration(minutes).ShouldBe(expected);
    }

    [Fact]
    public void Percent_Should_Round_And_Say_When_Not_Measured()
    {
        DisplayFormat.Percent(71.4).ShouldBe("71%");
        DisplayFormat.Percent(70.5).ShouldBe("71%");
        DisplayFormat.Percent(0).ShouldBe("0%");
        DisplayFormat.Percent(null).ShouldBe(DisplayFormat.NotMeasured);
        DisplayFormat.Percent(double.NaN).ShouldBe(DisplayFormat.NotMeasured);
    }

    [Fact]
    public void LocalTime_Should_Read_On_The_Zones_Clock()
    {
        var dhaka = ReportCalendar.ResolveZone("Asia/Dhaka");

        DisplayFormat.LocalTime(new DateTime(2026, 10, 24, 3, 12, 0, DateTimeKind.Utc), dhaka).ShouldBe("09:12");
        // Unspecified instants are stored UTC.
        DisplayFormat.LocalTime(new DateTime(2026, 10, 24, 20, 5, 0), dhaka).ShouldBe("02:05");
    }

    [Fact]
    public void Day_Should_Name_The_Weekday_And_The_Year_On_Request()
    {
        DisplayFormat.Day(new DateTime(2026, 10, 23)).ShouldBe("Fri 23 Oct");
        DisplayFormat.Day(new DateTime(2025, 12, 31), withYear: true).ShouldBe("Wed 31 Dec 2025");
    }

    [Fact]
    public void NeedsYear_Should_Be_True_Across_Years_Or_Outside_The_Current_One()
    {
        var today = new DateTime(2026, 10, 26);

        DisplayFormat.NeedsYear(new DateTime(2026, 10, 1), new DateTime(2026, 10, 25), today).ShouldBeFalse();
        DisplayFormat.NeedsYear(new DateTime(2025, 12, 20), new DateTime(2026, 1, 5), today).ShouldBeTrue();
        DisplayFormat.NeedsYear(new DateTime(2025, 3, 1), new DateTime(2025, 3, 31), today).ShouldBeTrue();
    }

    [Fact]
    public void Zone_Should_Name_The_Zone_And_Its_Offset_At_That_Instant()
    {
        DisplayFormat.Zone(ReportCalendar.ResolveZone("Asia/Dhaka"), new DateTime(2026, 10, 24, 0, 0, 0, DateTimeKind.Utc))
            .ShouldBe("Asia/Dhaka (UTC+06:00)");
        DisplayFormat.Zone(ReportCalendar.ResolveZone("Asia/Kolkata"), new DateTime(2026, 10, 24, 0, 0, 0, DateTimeKind.Utc))
            .ShouldBe("Asia/Kolkata (UTC+05:30)");

        var newYork = ReportCalendar.ResolveZone("America/New_York");
        DisplayFormat.Zone(newYork, new DateTime(2026, 1, 15, 12, 0, 0, DateTimeKind.Utc)).ShouldBe("America/New_York (UTC-05:00)");
        DisplayFormat.Zone(newYork, new DateTime(2026, 7, 15, 12, 0, 0, DateTimeKind.Utc)).ShouldBe("America/New_York (UTC-04:00)");

        DisplayFormat.Zone(TimeZoneInfo.Utc, new DateTime(2026, 7, 15)).ShouldBe("UTC (UTC+00:00)");
    }

    [Theory]
    [InlineData(42, "+42 min")]
    [InlineData(-65, "-1 h 5 min")]
    [InlineData(0.2, DisplayFormat.NoChange)]
    [InlineData(-0.4, DisplayFormat.NoChange)]
    [InlineData(double.NaN, DisplayFormat.NoChange)]
    public void Delta_Should_Carry_Its_Sign(double minutes, string expected)
    {
        DisplayFormat.Delta(minutes).ShouldBe(expected);
    }

    [Fact]
    public void DeltaPercent_Should_State_The_Relative_Change_Or_Nothing()
    {
        DisplayFormat.DeltaPercent(112, 100).ShouldBe("+12%");
        DisplayFormat.DeltaPercent(70, 100).ShouldBe("-30%");
        DisplayFormat.DeltaPercent(100.2, 100).ShouldBe(DisplayFormat.NoChange);
        DisplayFormat.DeltaPercent(5, 0).ShouldBeNull();
        DisplayFormat.DeltaPercent(null, 10).ShouldBeNull();
        DisplayFormat.DeltaPercent(10, null).ShouldBeNull();
    }
}
