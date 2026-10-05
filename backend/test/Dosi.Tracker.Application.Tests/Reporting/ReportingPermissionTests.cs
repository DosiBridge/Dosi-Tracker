using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The reports as a caller who may see everyone's activity (Tracker.Activities.ViewAll) but whose Tracker.Reporting
/// permission was removed: team reports are refused, their own reports stay open.
/// </summary>
public abstract class ReportingPermissionTests<TStartupModule> : ReportingTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Reporting.Default);
    }

    private IEnumerable<Func<GetReportSummaryInput, Task>> EveryReport()
    {
        yield return Reporting.GetSummaryAsync;
        yield return Reporting.GetDailySeriesAsync;
        yield return Reporting.GetUserDailySeriesAsync;
        yield return Reporting.GetAppUsageAsync;
        yield return Reporting.GetAttendanceAsync;
        yield return Reporting.GetPayrollAsync;
        yield return Reporting.GetHourlyAsync;
        yield return Reporting.GetDevicesAsync;
    }

    [Fact]
    public async Task Team_Reports_Should_Be_Refused_Without_The_Reporting_Permission()
    {
        foreach (var report in EveryReport())
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => report(Range(Day, Day.AddDays(1))));

            var colleague = Range(Day, Day.AddDays(1));
            colleague.UserId = OtherUserId;
            await Should.ThrowAsync<AbpAuthorizationException>(() => report(colleague));
        }
    }

    [Fact]
    public async Task Own_Reports_Should_Stay_Open_Without_The_Reporting_Permission()
    {
        var project = await SeedMyProjectAsync();
        await AddMemberAsync(project, OtherUserId);
        await InsertAsync(Block(project, Day.AddHours(9), 30), Block(project, Day.AddHours(9), 20, OtherUserId));

        var mine = Range(Day, Day.AddDays(1));
        mine.UserId = CurrentUserId;

        foreach (var report in EveryReport())
        {
            await report(mine);
        }

        var summary = await Reporting.GetSummaryAsync(mine);
        summary.TotalTrackedMinutes.ShouldBe(30);
        summary.PerUser.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
        (await Reporting.GetAttendanceAsync(mine)).Rows.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
    }
}
