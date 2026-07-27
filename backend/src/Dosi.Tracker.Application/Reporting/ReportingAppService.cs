using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Reporting;

[Authorize]
public class ReportingAppService : TrackerAppService, IReportingAppService
{
    private readonly IRepository<Activity, Guid> _activityRepository;

    public ReportingAppService(IRepository<Activity, Guid> activityRepository)
    {
        _activityRepository = activityRepository;
    }

    public async Task<ReportSummaryDto> GetSummaryAsync(GetReportSummaryInput input)
    {
        var query = await _activityRepository.GetQueryableAsync();

        query = query.Where(a => a.StartedAt >= input.From && a.StartedAt < input.To);

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            // Regular members only report on themselves.
            var userId = CurrentUser.GetId();
            query = query.Where(a => a.UserId == userId);
        }

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
        var query = await _activityRepository.GetQueryableAsync();

        query = query.Where(a => a.StartedAt >= input.From && a.StartedAt < input.To);

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            var userId = CurrentUser.GetId();
            query = query.Where(a => a.UserId == userId);
        }

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
            var active = ParseApps(r.ActiveWindowsJson);
            var running = ParseApps(r.RunningProgramsJson);

            // The focused app (first active window) is credited with the block's time; other apps
            // present in the block count towards its reach but not its minutes (avoids double-counting).
            var focused = active.FirstOrDefault();
            if (!string.IsNullOrWhiteSpace(focused))
            {
                minutes[focused] = minutes.GetValueOrDefault(focused) + duration;
            }

            foreach (var app in active.Concat(running).Distinct(StringComparer.OrdinalIgnoreCase))
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

    /// <summary>The range + project + own-vs-all filter shared by every report query.</summary>
    private async Task<IQueryable<Activity>> ScopedQueryAsync(GetReportSummaryInput input)
    {
        var query = await _activityRepository.GetQueryableAsync();
        query = query.Where(a => a.StartedAt >= input.From && a.StartedAt < input.To);

        if (input.ProjectId.HasValue)
        {
            query = query.Where(a => a.ProjectId == input.ProjectId.Value);
        }

        if (!await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            var userId = CurrentUser.GetId();
            query = query.Where(a => a.UserId == userId);
        }

        return query;
    }

    /// <summary>Extract distinct application names from a stored window-info JSON array; tolerant of bad data.</summary>
    private static List<string> ParseApps(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return new List<string>();
        }

        try
        {
            var windows = JsonSerializer.Deserialize<List<WindowInfoDto>>(
                json, new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
            return windows?
                .Where(w => !string.IsNullOrWhiteSpace(w.AppName))
                .Select(w => w.AppName!)
                .ToList() ?? new List<string>();
        }
        catch (JsonException)
        {
            return new List<string>();
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
