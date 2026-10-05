using System.Collections.Generic;
using System.Threading.Tasks;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// Reporting API (/api/app/reporting/*). Every endpoint takes the same <see cref="GetReportSummaryInput"/> and the
/// same scoping: callers holding Tracker.Activities.ViewAll and Tracker.Reporting see the whole team (with ViewAll
/// but not Tracker.Reporting, only a request for their own data is answered; anything else is refused with 403);
/// members who are a client of projects see every member's time on those projects plus their own; everyone else
/// sees only their own data. Every "day" (and hour) is read on the requested time zone's clock, or on the member's
/// own clock with the "member" day basis; a block belongs, whole, to the day of its start.
/// </summary>
public interface IReportingAppService : IApplicationService
{
    /// <summary>Tracked-time, idle and activity rollups for a date range, per member, project and member × project.</summary>
    Task<ReportSummaryDto> GetSummaryAsync(GetReportSummaryInput input);

    /// <summary>Per-local-day series over the range. Days without activity are included with zeros so charts
    /// have a continuous axis.</summary>
    Task<List<DailyReportRowDto>> GetDailySeriesAsync(GetReportSummaryInput input);

    /// <summary>Focused-time-by-application rollup for the Apps &amp; URLs report, grouped by canonical app name.</summary>
    Task<List<AppUsageRowDto>> GetAppUsageAsync(GetReportSummaryInput input);

    /// <summary>Presence and worked-time per member of the reported projects, absent members included.</summary>
    Task<AttendanceReportDto> GetAttendanceAsync(GetReportSummaryInput input);

    /// <summary>GET /api/app/reporting/user-daily-series — tracked minutes per member per local day
    /// (only member-days with activity are returned, ordered by day then user).</summary>
    Task<List<UserDailyReportRowDto>> GetUserDailySeriesAsync(GetReportSummaryInput input);

    /// <summary>GET /api/app/reporting/payroll — time and pay per member and project, each block priced at the
    /// hourly rate in effect at its start. Requires Tracker.Team.Manage.</summary>
    Task<PayrollReportDto> GetPayrollAsync(GetReportSummaryInput input);

    /// <summary>GET /api/app/reporting/hourly — tracked and active minutes per weekday and clock hour (all 168
    /// cells, Sunday 00:00 first); blocks are split across hour boundaries.</summary>
    Task<List<HourlyReportRowDto>> GetHourlyAsync(GetReportSummaryInput input);

    /// <summary>GET /api/app/reporting/devices — tracked time per member and device over the range.</summary>
    Task<List<DeviceReportRowDto>> GetDevicesAsync(GetReportSummaryInput input);
}
