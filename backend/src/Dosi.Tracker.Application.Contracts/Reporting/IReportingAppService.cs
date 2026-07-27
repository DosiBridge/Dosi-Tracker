using System.Threading.Tasks;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Reporting;

public interface IReportingAppService : IApplicationService
{
    /// <summary>Tracked-time and productivity rollups for a date range.
    /// Members without Tracker.Activities.ViewAll only ever see their own data.</summary>
    Task<ReportSummaryDto> GetSummaryAsync(GetReportSummaryInput input);

    /// <summary>Per-UTC-day series over the range (same scoping rules as the summary).
    /// Days without activity are included with zeros so charts have a continuous axis.</summary>
    Task<System.Collections.Generic.List<DailyReportRowDto>> GetDailySeriesAsync(GetReportSummaryInput input);

    /// <summary>Focused-time-by-application rollup for the Apps &amp; URLs report (same scoping rules).</summary>
    Task<System.Collections.Generic.List<AppUsageRowDto>> GetAppUsageAsync(GetReportSummaryInput input);

    /// <summary>Presence and worked-time per member, derived from tracked activity (same scoping rules).</summary>
    Task<AttendanceReportDto> GetAttendanceAsync(GetReportSummaryInput input);
}
