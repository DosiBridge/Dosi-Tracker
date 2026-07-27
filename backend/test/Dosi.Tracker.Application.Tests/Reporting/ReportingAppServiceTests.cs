using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Reporting;

public abstract class ReportingAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly IReportingAppService _reportingAppService;
    private readonly IActivityAppService _activityAppService;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly HashSet<Guid> _seededProjects = new();

    protected ReportingAppServiceTests()
    {
        _reportingAppService = GetRequiredService<IReportingAppService>();
        _activityAppService = GetRequiredService<IActivityAppService>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private static readonly DateTime Day = new(2026, 7, 20, 0, 0, 0, DateTimeKind.Utc);

    private async Task<ActivityDto> TrackAsync(Guid projectId, DateTime start, int minutes, int productivity)
    {
        if (_seededProjects.Add(projectId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(
                    new Project(projectId, null, "Seeded Project"));
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), null, projectId, CurrentUserId));
            });
        }

        return await _activityAppService.CreateAsync(new CreateActivityDto
        {
            ProjectId = projectId,
            ClientActivityId = Guid.NewGuid(),
            StartedAt = start,
            EndedAt = start.AddMinutes(minutes),
            Productivity = productivity,
            MouseClicks = 10,
            KeyboardHits = 20
        });
    }

    private async Task EnsureProjectAsync(Guid projectId)
    {
        if (_seededProjects.Add(projectId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(new Project(projectId, null, "Seeded Project"));
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), null, projectId, CurrentUserId));
            });
        }
    }

    private async Task TrackWithAppsAsync(
        Guid projectId, DateTime start, int minutes, string[] active, string[] running)
    {
        await EnsureProjectAsync(projectId);
        await _activityAppService.CreateAsync(new CreateActivityDto
        {
            ProjectId = projectId,
            ClientActivityId = Guid.NewGuid(),
            StartedAt = start,
            EndedAt = start.AddMinutes(minutes),
            Productivity = 80,
            MouseClicks = 1,
            KeyboardHits = 1,
            ActiveWindows = active.Select(a => new WindowInfoDto { AppName = a, WindowTitle = a }).ToList(),
            RunningPrograms = running.Select(a => new WindowInfoDto { AppName = a, WindowTitle = a }).ToList()
        });
    }

    [Fact]
    public async Task AppUsage_Should_Credit_Focused_Time_And_Count_Reach()
    {
        var projectId = Guid.NewGuid();
        // Block 1: focused on code.exe (30m), with chrome.exe also running in the background.
        await TrackWithAppsAsync(projectId, Day.AddHours(9), 30, new[] { "code.exe" }, new[] { "code.exe", "chrome.exe" });
        // Block 2: focused on chrome.exe (20m).
        await TrackWithAppsAsync(projectId, Day.AddHours(10), 20, new[] { "chrome.exe" }, new[] { "chrome.exe" });

        var apps = await _reportingAppService.GetAppUsageAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(1)
        });

        apps.Count.ShouldBe(2);
        apps[0].AppName.ShouldBe("code.exe"); // most focused minutes first
        apps[0].TrackedMinutes.ShouldBe(30);
        apps[0].ActivityCount.ShouldBe(1);
        apps[0].UserCount.ShouldBe(1);

        var chrome = apps.Single(a => a.AppName == "chrome.exe");
        chrome.TrackedMinutes.ShouldBe(20);       // focused only in block 2
        chrome.ActivityCount.ShouldBe(2);         // running in block 1, focused in block 2
    }

    [Fact]
    public async Task Attendance_Should_Derive_Present_Absent_Weekdays_And_Worked_Minutes()
    {
        var projectId = Guid.NewGuid();
        await TrackAsync(projectId, Day.AddHours(9), 30, 80);
        await TrackAsync(projectId, Day.AddHours(13), 30, 80);            // same day, second block
        await TrackAsync(projectId, Day.AddDays(1).AddHours(10), 45, 70); // next day

        var report = await _reportingAppService.GetAttendanceAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(3)
        });

        // Weekends are never counted as working days.
        report.Days.ShouldAllBe(d => d.DayOfWeek != DayOfWeek.Saturday && d.DayOfWeek != DayOfWeek.Sunday);

        report.Rows.Count.ShouldBe(1);
        var row = report.Rows[0];
        row.UserId.ShouldBe(CurrentUserId);
        row.DaysPresent.ShouldBe(2);     // two distinct days with activity
        row.WorkedMinutes.ShouldBe(105); // 30 + 30 + 45
        row.DaysAbsent.ShouldBe(report.Days.Count(d => d != Day.Date && d != Day.AddDays(1).Date));
        row.PresentDays.ShouldBe(new List<DateTime> { Day.Date, Day.AddDays(1).Date });
    }

    [Fact]
    public async Task Summary_Should_Use_Real_Durations_And_Duration_Weighted_Productivity()
    {
        var projectA = Guid.NewGuid();
        var projectB = Guid.NewGuid();

        // 30 min @ 100 and 10 min @ 60 on project A; 20 min @ 50 on project B.
        await TrackAsync(projectA, Day.AddHours(9), 30, 100);
        await TrackAsync(projectA, Day.AddHours(10), 10, 60);
        await TrackAsync(projectB, Day.AddHours(11), 20, 50);

        var summary = await _reportingAppService.GetSummaryAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(1)
        });

        summary.TotalActivities.ShouldBe(3);
        summary.TotalTrackedMinutes.ShouldBe(60);
        summary.TotalMouseClicks.ShouldBe(30);
        summary.TotalKeyboardHits.ShouldBe(60);

        // Weighted: (30*100 + 10*60 + 20*50) / 60 = (3000 + 600 + 1000) / 60 = 76.67
        summary.AverageProductivity.ShouldBe(76.67);

        summary.PerProject.Count.ShouldBe(2);
        var rowA = summary.PerProject.Single(p => p.ProjectId == projectA);
        rowA.ActivityCount.ShouldBe(2);
        rowA.TrackedMinutes.ShouldBe(40);
        rowA.AverageProductivity.ShouldBe(90); // (30*100 + 10*60) / 40

        var rowB = summary.PerProject.Single(p => p.ProjectId == projectB);
        rowB.TrackedMinutes.ShouldBe(20);
        rowB.AverageProductivity.ShouldBe(50);

        summary.PerUser.Count.ShouldBe(1); // single test user
        summary.PerUser[0].TrackedMinutes.ShouldBe(60);
    }

    [Fact]
    public async Task Summary_Should_Respect_Range_And_Project_Filters()
    {
        var projectA = Guid.NewGuid();
        var projectB = Guid.NewGuid();

        await TrackAsync(projectA, Day.AddHours(9), 30, 80);
        await TrackAsync(projectB, Day.AddHours(9), 15, 40);
        await TrackAsync(projectA, Day.AddDays(5), 60, 90); // outside range

        var summary = await _reportingAppService.GetSummaryAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(1),
            ProjectId = projectA
        });

        summary.TotalActivities.ShouldBe(1);
        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.AverageProductivity.ShouldBe(80);
        summary.PerProject.Count.ShouldBe(1);
        summary.PerProject[0].ProjectId.ShouldBe(projectA);
    }

    [Fact]
    public async Task DailySeries_Should_Cover_Every_Day_With_Zeros_For_Idle_Days()
    {
        var projectId = Guid.NewGuid();

        await TrackAsync(projectId, Day.AddHours(9), 30, 80);            // day 0
        await TrackAsync(projectId, Day.AddDays(2).AddHours(14), 45, 60); // day 2

        var series = await _reportingAppService.GetDailySeriesAsync(new GetReportSummaryInput
        {
            From = Day,
            To = Day.AddDays(3)
        });

        series.Count.ShouldBe(3);
        series[0].Date.ShouldBe(Day);
        series[0].TrackedMinutes.ShouldBe(30);
        series[0].AverageProductivity.ShouldBe(80);
        series[0].ActivityCount.ShouldBe(1);

        series[1].TrackedMinutes.ShouldBe(0); // idle day still on the axis
        series[1].ActivityCount.ShouldBe(0);

        series[2].TrackedMinutes.ShouldBe(45);
        series[2].AverageProductivity.ShouldBe(60);
        series[2].MouseClicks.ShouldBe(10);
        series[2].KeyboardHits.ShouldBe(20);
    }

    [Fact]
    public async Task Summary_Should_Be_Empty_For_A_Range_With_No_Activity()
    {
        var summary = await _reportingAppService.GetSummaryAsync(new GetReportSummaryInput
        {
            From = Day.AddYears(-1),
            To = Day.AddYears(-1).AddDays(1)
        });

        summary.TotalActivities.ShouldBe(0);
        summary.TotalTrackedMinutes.ShouldBe(0);
        summary.AverageProductivity.ShouldBe(0);
        summary.PerUser.ShouldBeEmpty();
        summary.PerProject.ShouldBeEmpty();
    }
}
