using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Volo.Abp;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Users;

namespace Dosi.Tracker.Reporting;

[Authorize]
public class ReportingAppService : TrackerAppService, IReportingAppService
{
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IIdentityUserRepository _userRepository;

    public ReportingAppService(
        IRepository<Activity, Guid> activityRepository,
        IIdentityUserRepository userRepository)
    {
        _activityRepository = activityRepository;
        _userRepository = userRepository;
    }

    public async Task<ReportSummaryDto> GetSummaryAsync(GetReportSummaryInput input)
    {
        var query = await ScopedQueryAsync(input);

        // Aggregate on a slim projection; block counts per range are modest (one per 5-60 min per user).
        var rows = await AsyncExecuter.ToListAsync(query.Select(a => new
        {
            a.UserId,
            a.ProjectId,
            a.StartedAt,
            a.EndedAt,
            a.Productivity,
            a.MouseClicks,
            a.KeyboardHits
        }));

        double MinutesOf(DateTime start, DateTime end) => (end - start).TotalMinutes;

        var summary = new ReportSummaryDto
        {
            TotalActivities = rows.Count,
            TotalTrackedMinutes = Math.Round(rows.Sum(r => MinutesOf(r.StartedAt, r.EndedAt)), 2),
            TotalMouseClicks = rows.Sum(r => (long)r.MouseClicks),
            TotalKeyboardHits = rows.Sum(r => (long)r.KeyboardHits),
            AverageProductivity = WeightedProductivity(
                rows.Select(r => (MinutesOf(r.StartedAt, r.EndedAt), (double)r.Productivity)))
        };

        summary.PerUser = rows
            .GroupBy(r => r.UserId)
            .Select(g => new UserReportRowDto
            {
                UserId = g.Key,
                ActivityCount = g.Count(),
                TrackedMinutes = Math.Round(g.Sum(r => MinutesOf(r.StartedAt, r.EndedAt)), 2),
                AverageProductivity = WeightedProductivity(
                    g.Select(r => (MinutesOf(r.StartedAt, r.EndedAt), (double)r.Productivity)))
            })
            .OrderByDescending(r => r.TrackedMinutes)
            .ToList();

        summary.PerProject = rows
            .GroupBy(r => r.ProjectId)
            .Select(g => new ProjectReportRowDto
            {
                ProjectId = g.Key,
                ActivityCount = g.Count(),
                TrackedMinutes = Math.Round(g.Sum(r => MinutesOf(r.StartedAt, r.EndedAt)), 2),
                AverageProductivity = WeightedProductivity(
                    g.Select(r => (MinutesOf(r.StartedAt, r.EndedAt), (double)r.Productivity)))
            })
            .OrderByDescending(r => r.TrackedMinutes)
            .ToList();

        return summary;
    }

    public async Task<List<DailyReportRowDto>> GetDailySeriesAsync(GetReportSummaryInput input)
    {
        var query = await ScopedQueryAsync(input);

        var rows = await AsyncExecuter.ToListAsync(query.Select(a => new
        {
            a.StartedAt,
            a.EndedAt,
            a.Productivity,
            a.MouseClicks,
            a.KeyboardHits
        }));

        var byDay = rows
            .GroupBy(r => r.StartedAt.Date)
            .ToDictionary(g => g.Key, g => g.ToList());

        // Continuous axis: emit every UTC day in the range, zeros included.
        var series = new List<DailyReportRowDto>();
        for (var day = input.From.Date; day < input.To; day = day.AddDays(1))
        {
            if (byDay.TryGetValue(day, out var dayRows))
            {
                series.Add(new DailyReportRowDto
                {
                    Date = day,
                    ActivityCount = dayRows.Count,
                    TrackedMinutes = Math.Round(dayRows.Sum(r => (r.EndedAt - r.StartedAt).TotalMinutes), 2),
                    AverageProductivity = WeightedProductivity(
                        dayRows.Select(r => ((r.EndedAt - r.StartedAt).TotalMinutes, (double)r.Productivity))),
                    MouseClicks = dayRows.Sum(r => (long)r.MouseClicks),
                    KeyboardHits = dayRows.Sum(r => (long)r.KeyboardHits)
                });
            }
            else
            {
                series.Add(new DailyReportRowDto { Date = day });
            }
        }

        return series;
    }

    public async Task<List<AppUsageRowDto>> GetAppUsageAsync(GetReportSummaryInput input)
    {
        var query = await ScopedQueryAsync(input);
        var rows = await AsyncExecuter.ToListAsync(query.Select(a => new
        {
            a.UserId,
            a.StartedAt,
            a.EndedAt,
            a.ActiveWindowsJson,
            a.RunningProgramsJson
        }));

        var minutes = new Dictionary<string, double>(StringComparer.OrdinalIgnoreCase);
        var activityCounts = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        var users = new Dictionary<string, HashSet<Guid>>(StringComparer.OrdinalIgnoreCase);

        foreach (var r in rows)
        {
            var duration = (r.EndedAt - r.StartedAt).TotalMinutes;
            var active = ParseWindows(r.ActiveWindowsJson);
            var running = ParseWindows(r.RunningProgramsJson);

            var timed = active.Where(w => w.Seconds > 0).ToList();
            if (timed.Count > 0)
            {
                // Newer agents report per-window foreground seconds: each app is credited with its own share
                // of the block. Over-reported totals are scaled down so the block is never credited more
                // minutes than it lasted.
                var reportedMinutes = timed.Sum(w => w.Seconds!.Value / 60.0);
                var scale = reportedMinutes > duration ? duration / reportedMinutes : 1;
                foreach (var window in timed)
                {
                    minutes[window.AppName!] = minutes.GetValueOrDefault(window.AppName!) + window.Seconds!.Value / 60.0 * scale;
                }
            }
            else
            {
                // Legacy blocks only carry the window focused at capture time: that app (first active window) is
                // credited with the block's time; other apps present in the block count towards their reach but
                // not their minutes (avoids double-counting).
                var focused = active.FirstOrDefault()?.AppName;
                if (!string.IsNullOrWhiteSpace(focused))
                {
                    minutes[focused] = minutes.GetValueOrDefault(focused) + duration;
                }
            }

            foreach (var app in active.Concat(running).Select(w => w.AppName!).Distinct(StringComparer.OrdinalIgnoreCase))
            {
                activityCounts[app] = activityCounts.GetValueOrDefault(app) + 1;
                if (!users.TryGetValue(app, out var set))
                {
                    set = new HashSet<Guid>();
                    users[app] = set;
                }
                set.Add(r.UserId);
            }
        }

        return activityCounts.Keys
            .Select(app => new AppUsageRowDto
            {
                AppName = app,
                TrackedMinutes = Math.Round(minutes.GetValueOrDefault(app), 2),
                ActivityCount = activityCounts[app],
                UserCount = users[app].Count
            })
            .OrderByDescending(a => a.TrackedMinutes)
            .ThenByDescending(a => a.ActivityCount)
            .ToList();
    }

    public async Task<AttendanceReportDto> GetAttendanceAsync(GetReportSummaryInput input)
    {
        var query = await ScopedQueryAsync(input);
        var rows = await AsyncExecuter.ToListAsync(query.Select(a => new
        {
            a.UserId,
            a.StartedAt,
            a.EndedAt
        }));

        var weekdays = new List<DateTime>();
        for (var day = input.From.Date; day < input.To; day = day.AddDays(1))
        {
            if (day.DayOfWeek != DayOfWeek.Saturday && day.DayOfWeek != DayOfWeek.Sunday)
            {
                weekdays.Add(day);
            }
        }

        var perUser = rows
            .GroupBy(r => r.UserId)
            .Select(g =>
            {
                var presentDays = g.Select(r => r.StartedAt.Date).ToHashSet();
                return new AttendanceRowDto
                {
                    UserId = g.Key,
                    DaysPresent = presentDays.Count,
                    DaysAbsent = weekdays.Count(d => !presentDays.Contains(d)),
                    WorkedMinutes = Math.Round(g.Sum(r => (r.EndedAt - r.StartedAt).TotalMinutes), 2),
                    PresentDays = presentDays.OrderBy(d => d).ToList()
                };
            })
            .OrderByDescending(r => r.WorkedMinutes)
            .ToList();

        return new AttendanceReportDto { Days = weekdays, Rows = perUser };
    }

    public async Task<List<UserDailyReportRowDto>> GetUserDailySeriesAsync(GetReportSummaryInput input)
    {
        var query = await ScopedQueryAsync(input);
        var rows = await AsyncExecuter.ToListAsync(query.Select(a => new
        {
            a.UserId,
            a.StartedAt,
            a.EndedAt
        }));

        var userIds = rows.Select(r => r.UserId).Distinct().ToList();
        var userNames = userIds.Count == 0
            ? new Dictionary<Guid, string>()
            : (await _userRepository.GetListByIdsAsync(userIds)).ToDictionary(u => u.Id, u => u.UserName);

        return rows
            .GroupBy(r => new { r.UserId, Day = r.StartedAt.Date })
            .Select(g => new UserDailyReportRowDto
            {
                UserId = g.Key.UserId,
                UserName = userNames.GetValueOrDefault(g.Key.UserId) ?? string.Empty,
                Date = DateTime.SpecifyKind(g.Key.Day, DateTimeKind.Utc),
                ActivityCount = g.Count(),
                TrackedMinutes = Math.Round(g.Sum(r => (r.EndedAt - r.StartedAt).TotalMinutes), 2)
            })
            .OrderBy(r => r.Date)
            .ThenBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.UserId)
            .ToList();
    }

    /// <summary>The range + project + user filter shared by every report query. Validates the range, and
    /// scopes callers without Tracker.Activities.ViewAll to their own data (their UserId input is ignored).</summary>
    private async Task<IQueryable<Activity>> ScopedQueryAsync(GetReportSummaryInput input)
    {
        EnsureValidRange(input);

        var query = await _activityRepository.GetQueryableAsync();
        query = query.Where(a => a.StartedAt >= input.From && a.StartedAt < input.To);

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            if (input.UserId.HasValue)
            {
                query = query.Where(a => a.UserId == input.UserId.Value);
            }
        }
        else
        {
            // Regular members only ever report on themselves.
            var userId = CurrentUser.GetId();
            query = query.Where(a => a.UserId == userId);
        }

        return query;
    }

    /// <summary>To must be after From, and the span is capped so a single request cannot scan (or, for the
    /// day-by-day reports, enumerate) an unbounded history.</summary>
    private static void EnsureValidRange(GetReportSummaryInput input)
    {
        if (input.To <= input.From || (input.To - input.From).TotalDays > GetReportSummaryInput.MaxRangeDays)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid)
                .WithData("maxDays", GetReportSummaryInput.MaxRangeDays)
                .WithData("from", input.From)
                .WithData("to", input.To);
        }
    }

    /// <summary>Extract the entries that name an application from a stored window-info JSON array (in stored
    /// order; per-window seconds when the agent reported them); tolerant of bad data.</summary>
    private static List<WindowInfoDto> ParseWindows(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return new List<WindowInfoDto>();
        }

        try
        {
            var windows = JsonSerializer.Deserialize<List<WindowInfoDto>>(
                json, new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
            return windows?
                .Where(w => w != null && !string.IsNullOrWhiteSpace(w.AppName))
                .ToList() ?? new List<WindowInfoDto>();
        }
        catch (JsonException)
        {
            return new List<WindowInfoDto>();
        }
    }

    /// <summary>Duration-weighted mean, so a 10-minute focused block outweighs a 1-minute idle one.</summary>
    private static double WeightedProductivity(IEnumerable<(double Minutes, double Productivity)> blocks)
    {
        double totalMinutes = 0, weighted = 0;
        foreach (var (minutes, productivity) in blocks)
        {
            totalMinutes += minutes;
            weighted += minutes * productivity;
        }

        return totalMinutes <= 0 ? 0 : Math.Round(weighted / totalMinutes, 2);
    }
}
