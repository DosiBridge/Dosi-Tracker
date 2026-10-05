using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Shouldly;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The calendar rules of the reports: local days of the requested zone (DST and quarter-hour offsets included),
/// the member day basis, hours on the local clock, and the attendance roster and absence rules.
/// </summary>
public abstract class ReportingCalendarRulesTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private const string NewYork = "America/New_York";
    private const string Kathmandu = "Asia/Kathmandu";
    private const string Dhaka = "Asia/Dhaka";

    private static readonly Guid AliceId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000a1");
    private static readonly Guid CarolId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000c1");
    private static readonly Guid DaveId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000d1");
    private static readonly Guid ErinId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000e1");
    private static readonly Guid FrankId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000f1");
    private static readonly Guid GinaId = Guid.Parse("7a1b2c3d-0000-4000-8000-0000000000b1");

    private static DateTime Utc(int year, int month, int day, int hour = 0, int minute = 0) =>
        new(year, month, day, hour, minute, 0, DateTimeKind.Utc);

    /// <summary>A local calendar date, as the day-based reports return it.</summary>
    private static DateTime Date(int year, int month, int day) => new(year, month, day);

    private static HourlyReportRowDto Cell(List<HourlyReportRowDto> rows, DayOfWeek day, int hour) =>
        rows.Single(r => r.DayOfWeek == (int)day && r.Hour == hour);

    // ---- Local days of the requested zone ----

    [Fact]
    public async Task DailySeries_Should_Use_The_Calendar_Days_Of_The_Requested_Zone()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Utc(2026, 7, 21, 2, 30), 30),  // Monday 22:30 in New York (EDT), Tuesday in UTC
            Block(projectId, Utc(2026, 7, 21, 13), 20));    // Tuesday 09:00 in New York

        // Monday and Tuesday in New York: [Mon 00:00 EDT, Wed 00:00 EDT).
        var newYork = await Reporting.GetDailySeriesAsync(Range(Utc(2026, 7, 20, 4), Utc(2026, 7, 22, 4), NewYork));

        newYork.Select(d => d.Date).ShouldBe(new[] { Date(2026, 7, 20), Date(2026, 7, 21) });
        newYork.ShouldAllBe(d => d.Date.Kind == DateTimeKind.Unspecified);
        newYork.Select(d => d.TrackedMinutes).ShouldBe(new[] { 30.0, 20.0 });

        var perMember = await Reporting.GetUserDailySeriesAsync(Range(Utc(2026, 7, 20, 4), Utc(2026, 7, 22, 4), NewYork));
        perMember.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[] { (Date(2026, 7, 20), 30.0), (Date(2026, 7, 21), 20.0) });

        // The same bounds read in UTC (the default): three partly covered UTC days, both blocks on Tuesday.
        var utc = await Reporting.GetDailySeriesAsync(Range(Utc(2026, 7, 20, 4), Utc(2026, 7, 22, 4)));
        utc.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[]
        {
            (Date(2026, 7, 20), 0.0), (Date(2026, 7, 21), 50.0), (Date(2026, 7, 22), 0.0)
        });
    }

    [Fact]
    public async Task DailySeries_Should_Follow_Daylight_Saving_Transitions()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            // Around the spring-forward Sunday (2026-03-08, a 23-hour day): each block starts at 23:30 local time.
            Block(projectId, Utc(2026, 3, 8, 4, 30), 20),   // Saturday 23:30 EST
            Block(projectId, Utc(2026, 3, 9, 3, 30), 20),   // Sunday 23:30 EDT
            Block(projectId, Utc(2026, 3, 10, 3, 30), 20),  // Monday 23:30 EDT
            // Around the fall-back Sunday (2025-11-02, a 25-hour day).
            Block(projectId, Utc(2025, 11, 2, 3, 30), 20),  // Saturday 23:30 EDT
            Block(projectId, Utc(2025, 11, 3, 4, 30), 20)); // Sunday 23:30 EST

        // [Sat 00:00 EST, Tue 00:00 EDT)
        var spring = await Reporting.GetDailySeriesAsync(Range(Utc(2026, 3, 7, 5), Utc(2026, 3, 10, 4), NewYork));
        spring.Select(d => (d.Date, d.ActivityCount)).ShouldBe(new[]
        {
            (Date(2026, 3, 7), 1), (Date(2026, 3, 8), 1), (Date(2026, 3, 9), 1)
        });

        // [Sat 00:00 EDT, Mon 00:00 EST)
        var fall = await Reporting.GetDailySeriesAsync(Range(Utc(2025, 11, 1, 4), Utc(2025, 11, 3, 5), NewYork));
        fall.Select(d => (d.Date, d.ActivityCount)).ShouldBe(new[] { (Date(2025, 11, 1), 1), (Date(2025, 11, 2), 1) });
    }

    [Fact]
    public async Task Reports_Should_Honour_A_Quarter_Hour_Offset()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Utc(2026, 7, 20, 18, 14), 1),  // Monday 23:59 in Kathmandu (+05:45)
            Block(projectId, Utc(2026, 7, 20, 18, 15), 10), // Tuesday 00:00
            Block(projectId, Utc(2026, 7, 21, 3), 60));     // Tuesday 08:45-09:45

        var range = Range(Utc(2026, 7, 19, 18, 15), Utc(2026, 7, 21, 18, 15), Kathmandu); // Mon + Tue in Kathmandu

        var days = await Reporting.GetDailySeriesAsync(range);
        days.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[] { (Date(2026, 7, 20), 1.0), (Date(2026, 7, 21), 70.0) });

        var hours = await Reporting.GetHourlyAsync(range);
        Cell(hours, DayOfWeek.Monday, 23).TrackedMinutes.ShouldBe(1);
        Cell(hours, DayOfWeek.Tuesday, 0).TrackedMinutes.ShouldBe(10);
        Cell(hours, DayOfWeek.Tuesday, 8).TrackedMinutes.ShouldBe(15);
        Cell(hours, DayOfWeek.Tuesday, 9).TrackedMinutes.ShouldBe(45);
        Cell(hours, DayOfWeek.Tuesday, 9).ActiveMinutes.ShouldBe(36); // 45 × 80%
        hours.Sum(h => h.TrackedMinutes).ShouldBe(71);
    }

    [Fact]
    public async Task Hourly_Should_Split_On_The_Local_Clock_Across_Daylight_Saving_Transitions()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Utc(2026, 3, 8, 6, 30), 60),  // 01:30 EST → 03:30 EDT: the 02:00 hour never happens
            Block(projectId, Utc(2025, 11, 2, 5), 120));   // 01:00 EDT → 02:00 EST: the 01:00 hour happens twice

        var spring = await Reporting.GetHourlyAsync(Range(Utc(2026, 3, 8), Utc(2026, 3, 9), NewYork));
        Cell(spring, DayOfWeek.Sunday, 1).TrackedMinutes.ShouldBe(30);
        Cell(spring, DayOfWeek.Sunday, 2).TrackedMinutes.ShouldBe(0);
        Cell(spring, DayOfWeek.Sunday, 3).TrackedMinutes.ShouldBe(30);

        var fall = await Reporting.GetHourlyAsync(Range(Utc(2025, 11, 2), Utc(2025, 11, 3), NewYork));
        Cell(fall, DayOfWeek.Sunday, 1).TrackedMinutes.ShouldBe(120);
        fall.Sum(h => h.TrackedMinutes).ShouldBe(120);
    }

    [Fact]
    public async Task Unknown_Zones_And_Day_Bases_Should_Fall_Back_To_The_Defaults()
    {
        var projectId = await SeedMyProjectAsync();
        // Monday 22:30 in New York, Tuesday 02:30 in UTC, Tuesday 12:30 on the device's own clock (+10:00).
        await InsertAsync(Block(projectId, Utc(2026, 7, 21, 2, 30), 30, configure: b => b.UtcOffsetMinutes = 600));

        async Task<DateTime> DayOf(string? timeZone, string? dayBasis = null)
        {
            var series = await Reporting.GetDailySeriesAsync(Range(Utc(2026, 7, 20, 4), Utc(2026, 7, 22, 4), timeZone, dayBasis));
            return series.Single(d => d.ActivityCount > 0).Date;
        }

        foreach (var unknown in new[] { null, "", "   ", "Mars/Olympus_Mons", new string('x', 200) })
        {
            (await DayOf(unknown)).ShouldBe(Date(2026, 7, 21)); // UTC
        }

        (await DayOf(NewYork)).ShouldBe(Date(2026, 7, 20));
        (await DayOf(NewYork, "galaxy")).ShouldBe(Date(2026, 7, 20));   // an unknown basis reads as "zone"
        (await DayOf(NewYork, "MEMBER")).ShouldBe(Date(2026, 7, 21));   // case-insensitive: the device's clock
    }

    // ---- Member day basis ----

    [Fact]
    public async Task Member_Basis_Should_Place_Each_Block_On_Its_Members_Own_Local_Day()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(
            Block(projectId, Utc(2026, 7, 20, 20), 30, configure: b => b.UtcOffsetMinutes = 360),   // Dhaka: Tue 02:00
            Block(projectId, Utc(2026, 7, 21, 2), 20, configure: b => b.TimeZone = NewYork),        // New York: Mon 22:00
            Block(projectId, Utc(2026, 7, 21, 12), 10),                                             // no device clock: UTC Tue
            Block(projectId, Utc(2026, 7, 20, 5), 15, configure: b => b.UtcOffsetMinutes = -600));  // Honolulu: Sun 19:00

        var range = Range(Utc(2026, 7, 20), Utc(2026, 7, 22), dayBasis: ReportDayBasis.Member);

        // The Honolulu block lands on its member's Sunday, outside the range's days: it is listed, so the series
        // still adds up to the range.
        var member = await Reporting.GetDailySeriesAsync(range);
        member.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[]
        {
            (Date(2026, 7, 19), 15.0), (Date(2026, 7, 20), 20.0), (Date(2026, 7, 21), 40.0)
        });

        var zone = await Reporting.GetDailySeriesAsync(Range(Utc(2026, 7, 20), Utc(2026, 7, 22)));
        zone.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[] { (Date(2026, 7, 20), 45.0), (Date(2026, 7, 21), 30.0) });

        // Totals never depend on the basis.
        (await Reporting.GetSummaryAsync(range)).TotalTrackedMinutes.ShouldBe(75);

        var perMember = await Reporting.GetUserDailySeriesAsync(range);
        perMember.Select(d => (d.Date, d.TrackedMinutes)).ShouldBe(new[]
        {
            (Date(2026, 7, 19), 15.0), (Date(2026, 7, 20), 20.0), (Date(2026, 7, 21), 40.0)
        });

        var attendance = await Reporting.GetAttendanceAsync(range);
        attendance.Days.ShouldBe(new[] { Date(2026, 7, 20), Date(2026, 7, 21) });
        var row = attendance.Rows.ShouldHaveSingleItem();
        row.PresentDays.ShouldBe(new[] { Date(2026, 7, 19), Date(2026, 7, 20), Date(2026, 7, 21) });
        row.DaysPresent.ShouldBe(2);
        row.WeekendDaysWorked.ShouldBe(1);
    }

    [Fact]
    public async Task Member_Basis_Should_Fall_Back_To_The_Requested_Zone_For_Blocks_Without_A_Device_Clock()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(Block(projectId, Utc(2026, 7, 20, 16), 30)); // Tuesday 01:00 in Tokyo, Monday in UTC

        var series = await Reporting.GetDailySeriesAsync(
            Range(Utc(2026, 7, 19, 15), Utc(2026, 7, 21, 15), "Asia/Tokyo", ReportDayBasis.Member));

        series.Single(d => d.ActivityCount > 0).Date.ShouldBe(Date(2026, 7, 21));
    }

    [Fact]
    public async Task Member_Basis_Should_Read_Hours_On_The_Members_Clock()
    {
        var projectId = await SeedMyProjectAsync();
        await InsertAsync(Block(projectId, Utc(2026, 7, 20, 3), 60, configure: b =>
        {
            b.UtcOffsetMinutes = 360;
            b.TimeZone = Dhaka;
        }));

        var range = Range(Utc(2026, 7, 20), Utc(2026, 7, 21));

        Cell(await Reporting.GetHourlyAsync(range), DayOfWeek.Monday, 3).TrackedMinutes.ShouldBe(60);

        range.DayBasis = ReportDayBasis.Member;
        var member = await Reporting.GetHourlyAsync(range);
        Cell(member, DayOfWeek.Monday, 9).TrackedMinutes.ShouldBe(60);
        Cell(member, DayOfWeek.Monday, 3).TrackedMinutes.ShouldBe(0);
    }

    // ---- Attendance ----

    [Fact]
    public async Task Attendance_Should_List_Absent_Members_But_Not_Clients_Inactive_Accounts_Or_Projects_Archived_Before_The_Range()
    {
        await CreateUserAsync(CurrentUserId, "me");
        await CreateUserAsync(AliceId, "alice");
        await CreateUserAsync(CarolId, "carol");
        await CreateUserAsync(DaveId, "dave", isActive: false);
        await CreateUserAsync(ErinId, "erin");
        await CreateUserAsync(FrankId, "frank");
        await DeleteUserAsync(FrankId);

        var project = await SeedMyProjectAsync("Website");
        await AddMemberAsync(project, AliceId);
        await AddMemberAsync(project, CarolId, TrackerRoles.ClientProjectRole);
        await AddMemberAsync(project, DaveId);
        await AddMemberAsync(project, FrankId); // a deleted account
        await AddMemberAsync(project, GinaId);  // a membership without an account
        var archived = await SeedProjectAsync("Old", archived: true); // archived months before the week
        await AddMemberAsync(archived, ErinId);

        await InsertAsync(Block(project, Day.AddHours(9), 30), Block(project, Day.AddDays(1).AddHours(10), 45));

        var week = Range(Day, Day.AddDays(5)); // Monday to Friday
        var report = await Reporting.GetAttendanceAsync(week);

        report.Days.Count.ShouldBe(5);
        report.Rows.Select(r => r.UserName).ShouldBe(new[] { "me", "alice" });

        var me = report.Rows[0];
        me.DaysPresent.ShouldBe(2);
        me.DaysAbsent.ShouldBe(3);
        me.WorkedMinutes.ShouldBe(75);

        var alice = report.Rows[1];
        alice.UserId.ShouldBe(AliceId);
        alice.DaysPresent.ShouldBe(0);
        alice.DaysAbsent.ShouldBe(5);
        alice.WorkedMinutes.ShouldBe(0);
        alice.AverageStartMinutes.ShouldBeNull();
        alice.AverageEndMinutes.ShouldBeNull();
        alice.PresentDays.ShouldBeEmpty();

        // Filters narrow the roster too.
        week.UserId = AliceId;
        (await Reporting.GetAttendanceAsync(week)).Rows.ShouldHaveSingleItem().UserId.ShouldBe(AliceId);

        week.UserId = null;
        week.ProjectId = archived;
        (await Reporting.GetAttendanceAsync(week)).Rows.ShouldBeEmpty();
    }

    [Fact]
    public async Task Attendance_Should_Count_Absences_On_A_Project_Archived_After_The_Range()
    {
        // The project was archived on the Wednesday after the reported week: that week its members were still expected.
        await CreateUserAsync(AliceId, "alice");
        await CreateUserAsync(CarolId, "carol");
        var project = await SeedProjectAsync("Website", archivedAt: Day.AddDays(9).AddHours(12));
        await AddMemberAsync(project, AliceId);
        await AddMemberAsync(project, CarolId);
        await InsertAsync(Block(project, Day.AddHours(9), 30, AliceId)); // Alice tracked on Monday only

        var week = Range(Day, Day.AddDays(5));
        week.ProjectId = project;
        var report = await Reporting.GetAttendanceAsync(week);

        report.Rows.Select(r => (r.UserName, r.DaysPresent, r.DaysAbsent)).ShouldBe(new[]
        {
            ("alice", 1, 4), ("carol", 0, 5)
        });

        // The team-wide report expects them too, although their only project is archived today.
        week.ProjectId = null;
        (await Reporting.GetAttendanceAsync(week)).Rows.Select(r => (r.UserName, r.DaysAbsent))
            .ShouldBe(new[] { ("alice", 4), ("carol", 5) });
    }

    [Fact]
    public async Task Attendance_Should_Stop_Counting_Absences_When_The_Project_Was_Archived()
    {
        // Archived on Wednesday at noon: Monday and Tuesday could be missed, Wednesday (cut short) and later could not.
        await CreateUserAsync(AliceId, "alice");
        await CreateUserAsync(CarolId, "carol");
        var archived = await SeedProjectAsync("Website", archivedAt: Day.AddDays(2).AddHours(12));
        await AddMemberAsync(archived, AliceId);
        await AddMemberAsync(archived, CarolId);
        // Carol also joined an active project on Thursday morning.
        var active = await SeedProjectAsync("Mobile app");
        await AddMemberAsync(active, CarolId, joinedAt: Day.AddDays(3).AddHours(8));
        await InsertAsync(Block(archived, Day.AddHours(9), 30, AliceId)); // Alice tracked on Monday

        var report = await Reporting.GetAttendanceAsync(Range(Day, Day.AddDays(5)));

        var alice = report.Rows.Single(r => r.UserId == AliceId);
        alice.DaysPresent.ShouldBe(1);
        alice.DaysAbsent.ShouldBe(1);   // Tuesday

        // Monday and Tuesday on the archived project; Friday on the new one (she joined on Thursday morning).
        report.Rows.Single(r => r.UserId == CarolId).DaysAbsent.ShouldBe(3);
    }

    [Fact]
    public async Task Attendance_Should_Show_The_Time_Of_Former_Members_Without_Counting_Absences()
    {
        await CreateUserAsync(CarolId, "carol");
        await CreateUserAsync(DaveId, "dave");
        var project = await SeedProjectAsync("Website");
        await AddMemberAsync(project, CarolId, "client"); // became a client after tracking
        // Dave was removed from the project after tracking: no membership left.

        await InsertAsync(Block(project, Day.AddHours(9), 30, CarolId), Block(project, Day.AddHours(9), 20, DaveId));

        var report = await Reporting.GetAttendanceAsync(Range(Day, Day.AddDays(5)));

        report.Rows.Select(r => (r.UserName, r.WorkedMinutes, r.DaysPresent, r.DaysAbsent)).ShouldBe(new[]
        {
            ("carol", 30.0, 1, 0), ("dave", 20.0, 1, 0)
        });
    }

    [Fact]
    public async Task Attendance_Should_Count_Weekdays_Only_And_Absences_Only_After_A_Member_Joined()
    {
        await CreateUserAsync(FrankId, "frank");
        var project = await SeedMyProjectAsync();
        await AddMemberAsync(project, FrankId, joinedAt: Day.AddDays(2).AddHours(10)); // Wednesday 10:00

        await InsertAsync(
            Block(project, Day.AddHours(9), 30),                 // Monday
            Block(project, Day.AddDays(1).AddHours(9), 30),      // Tuesday
            Block(project, Day.AddDays(5).AddHours(11), 20));    // Saturday

        var report = await Reporting.GetAttendanceAsync(Range(Day, Day.AddDays(7))); // a whole week

        report.Days.ShouldBe(Enumerable.Range(0, 5).Select(i => Date(2026, 7, 20 + i)));

        var me = report.Rows.Single(r => r.UserId == CurrentUserId);
        me.DaysPresent.ShouldBe(2);           // weekdays only
        me.WeekendDaysWorked.ShouldBe(1);
        me.DaysAbsent.ShouldBe(3);            // Wednesday to Friday
        me.PresentDays.ShouldBe(new[] { Date(2026, 7, 20), Date(2026, 7, 21), Date(2026, 7, 25) });

        // Frank joined on Wednesday morning: only Thursday and Friday were his to miss.
        report.Rows.Single(r => r.UserId == FrankId).DaysAbsent.ShouldBe(2);
    }

    [Fact]
    public async Task Attendance_Should_Not_Count_A_Partly_Covered_Day_As_Missed()
    {
        // A Dhaka viewer opens "Today" on Tuesday at 15:00: [Tue 00:00 +06, Tue 15:00 +06) = [Mon 18:00Z, Tue 09:00Z).
        await CreateUserAsync(AliceId, "alice");
        var project = await SeedMyProjectAsync();
        await AddMemberAsync(project, AliceId);
        await InsertAsync(Block(project, Utc(2026, 7, 21, 3), 30)); // Tuesday 09:00 in Dhaka

        var report = await Reporting.GetAttendanceAsync(Range(Utc(2026, 7, 20, 18), Utc(2026, 7, 21, 9), Dhaka));

        report.Days.ShouldBe(new[] { Date(2026, 7, 21) }); // never the UTC Monday the range starts on
        var me = report.Rows.Single(r => r.UserId == CurrentUserId);
        me.DaysPresent.ShouldBe(1);
        me.DaysAbsent.ShouldBe(0);
        report.Rows.Single(r => r.UserId == AliceId).DaysAbsent.ShouldBe(0); // Tuesday is not over in the range
    }

    [Fact]
    public async Task Attendance_Should_Never_Count_Future_Days_As_Missed()
    {
        await CreateUserAsync(AliceId, "alice");
        var project = await SeedMyProjectAsync();
        await AddMemberAsync(project, AliceId);

        var today = DateTime.SpecifyKind(TestClock.Now.ToUniversalTime().Date, DateTimeKind.Utc);
        var report = await Reporting.GetAttendanceAsync(Range(today.AddDays(-3), today.AddDays(4)));

        bool IsWeekday(DateTime day) => day.DayOfWeek is not (DayOfWeek.Saturday or DayOfWeek.Sunday);
        var week = Enumerable.Range(-3, 7).Select(i => today.AddDays(i)).ToList();

        report.Days.ShouldBe(week.Where(IsWeekday).Select(d => DateTime.SpecifyKind(d, DateTimeKind.Unspecified)));
        report.Rows.Single(r => r.UserId == AliceId).DaysAbsent.ShouldBe(week.Count(d => d <= today && IsWeekday(d)));
    }

    [Fact]
    public async Task Attendance_Should_Average_The_First_Start_And_Last_End_In_Local_Minutes()
    {
        var project = await SeedMyProjectAsync();
        await InsertAsync(
            Block(project, Utc(2026, 7, 20, 13), 30),        // Mon 09:00-09:30 EDT
            Block(project, Utc(2026, 7, 20, 21), 30),        // Mon 17:00-17:30
            Block(project, Utc(2026, 7, 21, 14), 45),        // Tue 10:00-10:45
            Block(project, Utc(2026, 7, 24, 3, 50), 20),     // Thu 23:50 - Fri 00:10: counts on Thursday, ends past midnight
            Block(project, Utc(2026, 7, 25, 15), 20));       // Sat 11:00-11:20

        // Monday to Sunday in New York.
        var report = await Reporting.GetAttendanceAsync(Range(Utc(2026, 7, 20, 4), Utc(2026, 7, 27, 4), NewYork));

        var me = report.Rows.ShouldHaveSingleItem();
        me.PresentDays.ShouldBe(new[] { Date(2026, 7, 20), Date(2026, 7, 21), Date(2026, 7, 23), Date(2026, 7, 25) });
        me.DaysPresent.ShouldBe(3);
        me.WeekendDaysWorked.ShouldBe(1);
        me.DaysAbsent.ShouldBe(2);                       // Wednesday and Friday
        me.AverageStartMinutes.ShouldBe(807.5);          // (540 + 600 + 1430 + 660) / 4
        me.AverageEndMinutes.ShouldBe(956.25);           // (1050 + 645 + 1450 + 680) / 4
        me.WorkedMinutes.ShouldBe(145);
    }

    [Fact]
    public async Task Attendance_Should_Report_Idle_Minutes()
    {
        var project = await SeedMyProjectAsync();
        await InsertAsync(
            Block(project, Day.AddHours(9), 30, configure: b => b.IdleSeconds = 300),
            Block(project, Day.AddHours(10), 30, configure: b => b.IdleSeconds = 30));

        var me = (await Reporting.GetAttendanceAsync(Range(Day, Day.AddDays(1)))).Rows.ShouldHaveSingleItem();

        me.IdleMinutes.ShouldBe(5.5);
        me.WorkedMinutes.ShouldBe(60);
    }
}
