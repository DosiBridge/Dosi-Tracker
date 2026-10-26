using System;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.AiAssist.Read;

public class AiPeriodResolverTests
{
    private const string Kathmandu = "Asia/Kathmandu"; // UTC+05:45, no DST
    private const string NewYork = "America/New_York";

    /// <summary>Monday 2026-10-26, 03:00 UTC (08:45 in Kathmandu).</summary>
    private static readonly DateTime Now = new(2026, 10, 26, 3, 0, 0, DateTimeKind.Utc);

    private static DateTime Utc(int year, int month, int day, int hour = 0, int minute = 0) =>
        new(year, month, day, hour, minute, 0, DateTimeKind.Utc);

    private static DateTime Local(int year, int month, int day) => new(year, month, day);

    [Fact]
    public void Today_In_A_Quarter_Hour_Zone_Should_Start_At_Its_Local_Midnight()
    {
        var today = AiPeriodResolver.Resolve("today", Kathmandu, Now);

        today.FromUtc.ShouldBe(Utc(2026, 10, 25, 18, 15));
        today.ToUtc.ShouldBe(Utc(2026, 10, 26, 18, 15));
        today.FromUtc.Kind.ShouldBe(DateTimeKind.Utc);
        today.FirstDay.ShouldBe(Local(2026, 10, 26));
        today.LastDay.ShouldBe(Local(2026, 10, 26));
        today.Days.ShouldBe(1);
        today.TimeZone.ShouldBe(Kathmandu);
    }

    [Fact]
    public void The_Local_Day_Should_Decide_Today_Not_The_Utc_One()
    {
        // 20:00 UTC on the 25th is already 01:45 on the 26th in Kathmandu.
        AiPeriodResolver.Resolve("today", Kathmandu, Utc(2026, 10, 25, 20)).FirstDay.ShouldBe(Local(2026, 10, 26));
        AiPeriodResolver.Resolve("today", "UTC", Utc(2026, 10, 25, 20)).FirstDay.ShouldBe(Local(2026, 10, 25));
    }

    [Fact]
    public void Weeks_Should_Run_Monday_To_Sunday()
    {
        var thisWeek = AiPeriodResolver.Resolve("this_week", Kathmandu, Now);
        thisWeek.FirstDay.ShouldBe(Local(2026, 10, 26));
        thisWeek.LastDay.ShouldBe(Local(2026, 10, 26));

        var lastWeek = AiPeriodResolver.Resolve("last_week", Kathmandu, Now);
        lastWeek.FirstDay.ShouldBe(Local(2026, 10, 19));
        lastWeek.LastDay.ShouldBe(Local(2026, 10, 25));
        lastWeek.Days.ShouldBe(7);
        lastWeek.FromUtc.ShouldBe(Utc(2026, 10, 18, 18, 15));
        lastWeek.ToUtc.ShouldBe(Utc(2026, 10, 25, 18, 15));

        // On a Sunday the week began six days earlier.
        var sunday = AiPeriodResolver.Resolve("this_week", "UTC", Utc(2026, 10, 25, 12));
        sunday.FirstDay.ShouldBe(Local(2026, 10, 19));
        sunday.LastDay.ShouldBe(Local(2026, 10, 25));
    }

    [Theory]
    [InlineData("yesterday", 2026, 10, 25, 2026, 10, 25)]
    [InlineData("last_7_days", 2026, 10, 20, 2026, 10, 26)]
    [InlineData("this_month", 2026, 10, 1, 2026, 10, 26)]
    [InlineData("last_month", 2026, 9, 1, 2026, 9, 30)]
    [InlineData("last_30_days", 2026, 9, 27, 2026, 10, 26)]
    [InlineData("This_Week", 2026, 10, 26, 2026, 10, 26)]
    public void Named_Periods_Should_Resolve_To_Whole_Local_Days(
        string period, int fromYear, int fromMonth, int fromDay, int toYear, int toMonth, int toDay)
    {
        var resolved = AiPeriodResolver.Resolve(period, "UTC", Now);

        resolved.FirstDay.ShouldBe(Local(fromYear, fromMonth, fromDay));
        resolved.LastDay.ShouldBe(Local(toYear, toMonth, toDay));
        resolved.FromUtc.ShouldBe(Utc(fromYear, fromMonth, fromDay));
        resolved.ToUtc.ShouldBe(Utc(toYear, toMonth, toDay).AddDays(1));
    }

    [Fact]
    public void A_Day_When_Clocks_Go_Back_Should_Last_Twenty_Five_Hours()
    {
        var day = AiPeriodResolver.Resolve("custom", NewYork, Now, "2026-11-01", "2026-11-01");

        day.FromUtc.ShouldBe(Utc(2026, 11, 1, 4)); // midnight EDT
        day.ToUtc.ShouldBe(Utc(2026, 11, 2, 5)); // midnight EST
        (day.ToUtc - day.FromUtc).ShouldBe(TimeSpan.FromHours(25));
    }

    [Fact]
    public void A_Day_When_Clocks_Go_Forward_Should_Last_Twenty_Three_Hours()
    {
        var day = AiPeriodResolver.ResolveDay("2026-03-08", NewYork);

        day.FromUtc.ShouldBe(Utc(2026, 3, 8, 5)); // midnight EST
        day.ToUtc.ShouldBe(Utc(2026, 3, 9, 4)); // midnight EDT
        day.FirstDay.ShouldBe(Local(2026, 3, 8));
    }

    [Fact]
    public void A_Week_Across_A_Dst_Change_Should_Still_Be_Seven_Local_Days()
    {
        var week = AiPeriodResolver.Resolve("last_week", NewYork, Utc(2026, 11, 3, 15));

        week.FirstDay.ShouldBe(Local(2026, 10, 26));
        week.LastDay.ShouldBe(Local(2026, 11, 1));
        week.FromUtc.ShouldBe(Utc(2026, 10, 26, 4));
        week.ToUtc.ShouldBe(Utc(2026, 11, 2, 5));
    }

    [Fact]
    public void Custom_Ranges_Should_Be_Inclusive_And_Capped()
    {
        var quarter = AiPeriodResolver.Resolve("custom", "UTC", Now, "2026-01-01", "2026-04-02");
        quarter.Days.ShouldBe(AiAssistConsts.MaxAssistantRangeDays);

        var tooLong = Should.Throw<BusinessException>(() => AiPeriodResolver.Resolve("custom", "UTC", Now, "2026-01-01", "2026-04-03"));
        tooLong.Code.ShouldBe(TrackerDomainErrorCodes.AiRangeTooLarge);
        tooLong.Data["maxDays"].ShouldBe(AiAssistConsts.MaxAssistantRangeDays);

        Should.Throw<BusinessException>(() => AiPeriodResolver.Resolve("custom", "UTC", Now, "2026-02-02", "2026-02-01"))
            .Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
    }

    [Theory]
    [InlineData("fortnight", null, null)]
    [InlineData(null, null, null)]
    [InlineData("custom", null, "2026-01-02")]
    [InlineData("custom", "2026-01-01", "02/01/2026")]
    [InlineData("custom", "2026-13-01", "2026-12-01")]
    [InlineData("custom", "0001-01-01", "0001-01-02")]
    [InlineData("custom", "9999-12-30", "9999-12-31")]
    public void Unknown_Periods_And_Malformed_Dates_Should_Be_Invalid(string? period, string? from, string? to)
    {
        Should.Throw<AbpValidationException>(() => AiPeriodResolver.Resolve(period, "UTC", Now, from, to));
    }

    [Fact]
    public void An_Unknown_Zone_Should_Read_As_Utc()
    {
        var today = AiPeriodResolver.Resolve("today", "Mars/Olympus_Mons", Now);

        today.TimeZone.ShouldBe(TimeZoneInfo.Utc.Id);
        today.FromUtc.ShouldBe(Utc(2026, 10, 26));
        Should.Throw<AbpValidationException>(() => AiPeriodResolver.ResolveDay("26 Oct", "UTC"));
    }
}
