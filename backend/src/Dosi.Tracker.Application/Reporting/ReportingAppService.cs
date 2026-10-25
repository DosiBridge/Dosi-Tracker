using System;
using System.Collections.Generic;
using System.Linq;
using System.Linq.Expressions;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Users;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// Reporting over tracked blocks.
/// <para>
/// Scope (every endpoint): callers holding Tracker.Activities.ViewAll and Tracker.Reporting see the whole team; a
/// ViewAll caller without Tracker.Reporting may only report on themselves (asking for the team is refused rather
/// than silently answered with their own figures); everyone else — workers, and members who are a client of some
/// projects — sees their own data, plus, for clients, every member's time on the projects they are a client of.
/// Nobody is ever refused their own reports.
/// </para>
/// <para>
/// Days: see <see cref="ReportCalendar"/> — a block belongs, whole, to the local day of its start in the chosen
/// basis (the requested zone, or the member's own clock).
/// </para>
/// <para>
/// Cost: every report reads one narrow projection of the range's blocks (the (TenantId, StartedAt) index bounds the
/// scan) and folds it into per-group totals as the rows stream in, so memory grows with the number of groups, not
/// blocks. Names, titles, rates and the roster are each loaded in one batched query. Durations and local days are
/// computed here rather than in SQL, because they need DST-aware time zones and per-block clocks.
/// </para>
/// </summary>
[Authorize]
public class ReportingAppService : TrackerAppService, IReportingAppService
{
    private static readonly JsonSerializerOptions StoredJsonOptions = new() { PropertyNameCaseInsensitive = true };

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly IIdentityUserRepository _userRepository;

    public ReportingAppService(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<ProjectMemberRate, Guid> rateRepository,
        IIdentityUserRepository userRepository)
    {
        _activityRepository = activityRepository;
        _projectRepository = projectRepository;
        _memberRepository = memberRepository;
        _rateRepository = rateRepository;
        _userRepository = userRepository;
    }

    public async Task<ReportSummaryDto> GetSummaryAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var activities = await ScopedActivitiesAsync(scope);

        // Pay rates are Team.Manage data (as on the project roster): without it the amounts stay 0 and no rate is read.
        var rates = await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage)
            ? await RateSchedulesAsync(activities)
            : null;

        var total = new BlockTotals();
        var perUser = new Dictionary<Guid, BlockTotals>();
        var perProject = new Dictionary<Guid, BlockTotals>();
        var perUserProject = new Dictionary<(Guid UserId, Guid ProjectId), BlockTotals>();

        await foreach (var block in StreamAsync(activities.Select(BlockProjection(withDeviceClock: false))))
        {
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);
            var idle = IdleMinutesOf(block.IdleSeconds, minutes);

            total.Add(minutes, block.Productivity, idle, block.MouseClicks, block.KeyboardHits);
            TotalsOf(perUser, block.UserId).Add(minutes, block.Productivity, idle);
            TotalsOf(perProject, block.ProjectId).Add(minutes, block.Productivity, idle);

            var pair = TotalsOf(perUserProject, (block.UserId, block.ProjectId));
            pair.Add(minutes, block.Productivity, idle);
            if (rates != null)
            {
                pair.AddPay(minutes, RateAt(rates, block.UserId, block.ProjectId, block.StartedAt));
            }
        }

        var users = await UsersAsync(perUser.Keys);
        var projects = await ProjectsAsync(perProject.Keys);

        return new ReportSummaryDto
        {
            TotalActivities = total.Count,
            TotalTrackedMinutes = total.RoundedMinutes,
            TotalIdleMinutes = total.RoundedIdleMinutes,
            AverageProductivity = total.AverageProductivity,
            TotalMouseClicks = total.MouseClicks,
            TotalKeyboardHits = total.KeyboardHits,
            MouseMeasuredActivities = total.MouseMeasuredCount,
            KeyboardMeasuredActivities = total.KeyboardMeasuredCount,
            PerUser = perUser
                .Select(entry => new UserReportRowDto
                {
                    UserId = entry.Key,
                    UserName = UserNameOf(users, entry.Key),
                    Name = FullNameOf(users.GetValueOrDefault(entry.Key)),
                    ActivityCount = entry.Value.Count,
                    TrackedMinutes = entry.Value.RoundedMinutes,
                    IdleMinutes = entry.Value.RoundedIdleMinutes,
                    AverageProductivity = entry.Value.AverageProductivity
                })
                .OrderByDescending(r => r.TrackedMinutes)
                .ThenBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
                .ThenBy(r => r.UserId)
                .ToList(),
            PerProject = perProject
                .Select(entry =>
                {
                    var project = projects.GetValueOrDefault(entry.Key);
                    return new ProjectReportRowDto
                    {
                        ProjectId = entry.Key,
                        ProjectTitle = project?.Title ?? string.Empty,
                        IsArchived = project?.IsArchived ?? false,
                        ActivityCount = entry.Value.Count,
                        TrackedMinutes = entry.Value.RoundedMinutes,
                        IdleMinutes = entry.Value.RoundedIdleMinutes,
                        AverageProductivity = entry.Value.AverageProductivity
                    };
                })
                .OrderByDescending(r => r.TrackedMinutes)
                .ThenBy(r => r.ProjectTitle, StringComparer.OrdinalIgnoreCase)
                .ThenBy(r => r.ProjectId)
                .ToList(),
            PerUserProject = UserProjectRows(perUserProject, users, projects, withPay: rates != null)
        };
    }

    public async Task<List<DailyReportRowDto>> GetDailySeriesAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var calendar = ReportCalendar.For(input);
        var activities = await ScopedActivitiesAsync(scope);

        var perDay = new Dictionary<DateTime, BlockTotals>();
        await foreach (var block in StreamAsync(activities.Select(BlockProjection(calendar.MemberBasis))))
        {
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);
            TotalsOf(perDay, DayOf(calendar, block)).Add(
                minutes, block.Productivity, IdleMinutesOf(block.IdleSeconds, minutes), block.MouseClicks, block.KeyboardHits);
        }

        // Continuous axis: every local day of the range, zeros included. With the member basis a block can land on a
        // member's local day just outside it; such a day is listed too, so the series always adds up to the range.
        var days = calendar.DaysOverlapping(scope.From, scope.To);
        days.AddRange(perDay.Keys.Except(days).ToList());
        days.Sort();

        return days
            .Select(day => perDay.TryGetValue(day, out var totals)
                ? new DailyReportRowDto
                {
                    Date = day,
                    ActivityCount = totals.Count,
                    TrackedMinutes = totals.RoundedMinutes,
                    IdleMinutes = totals.RoundedIdleMinutes,
                    AverageProductivity = totals.AverageProductivity,
                    MouseClicks = totals.MouseClicks,
                    KeyboardHits = totals.KeyboardHits,
                    MouseMeasuredActivities = totals.MouseMeasuredCount,
                    KeyboardMeasuredActivities = totals.KeyboardMeasuredCount
                }
                : new DailyReportRowDto { Date = day })
            .ToList();
    }

    public async Task<List<AppUsageRowDto>> GetAppUsageAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var activities = await ScopedActivitiesAsync(scope);

        // Only the window lists are needed, and the minute timeline only of blocks that were partly idle (it places
        // their idle minutes on the app in front); everything else stays in the database.
        var rows = activities.Select(a => new AppUsageBlock
        {
            UserId = a.UserId,
            StartedAt = a.StartedAt,
            EndedAt = a.EndedAt,
            IdleSeconds = a.IdleSeconds,
            ActiveWindowsJson = a.ActiveWindowsJson,
            RunningProgramsJson = a.RunningProgramsJson,
            TimelineJson = a.IdleSeconds > 0 ? a.TimelineJson : null
        });

        var apps = new Dictionary<string, AppUsageTotals>(StringComparer.Ordinal);
        await foreach (var block in StreamAsync(rows))
        {
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);
            var active = ParseApps(block.ActiveWindowsJson);
            var running = ParseApps(block.RunningProgramsJson);

            var credit = CreditFocusedTime(active, minutes);
            var idle = IdleByApp(IdleMinutesOf(block.IdleSeconds, minutes), block.TimelineJson, minutes, credit);

            foreach (var (key, spellings) in credit)
            {
                var app = AppTotalsOf(apps, key);
                foreach (var (raw, credited) in spellings)
                {
                    app.Credit(raw, credited);
                }

                app.IdleMinutes += idle.GetValueOrDefault(key);
            }

            // Reach: every app present in the block (focused or merely running) counts once per block.
            foreach (var entry in active.Concat(running).DistinctBy(e => e.Key))
            {
                var app = AppTotalsOf(apps, entry.Key);
                app.ActivityCount++;
                app.Users.Add(block.UserId);
                app.Credit(entry.Raw, 0);
            }
        }

        return apps.Values
            .Select(app =>
            {
                var appName = app.MostCreditedSpelling();
                return new AppUsageRowDto
                {
                    AppName = appName,
                    DisplayName = AppNames.Canonical(appName),
                    TrackedMinutes = Math.Round(app.Minutes, 2),
                    IdleMinutes = Math.Round(Math.Min(app.IdleMinutes, app.Minutes), 2),
                    ActivityCount = app.ActivityCount,
                    UserCount = app.Users.Count
                };
            })
            .OrderByDescending(a => a.TrackedMinutes)
            .ThenByDescending(a => a.ActivityCount)
            .ThenBy(a => a.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    public async Task<AttendanceReportDto> GetAttendanceAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var calendar = ReportCalendar.For(input);
        var activities = await ScopedActivitiesAsync(scope);
        var roster = await RosterAsync(scope);

        var perMember = new Dictionary<Guid, MemberAttendance>();
        await foreach (var block in StreamAsync(activities.Select(BlockProjection(calendar.MemberBasis))))
        {
            var clock = calendar.ClockFor(block.TimeZone, block.UtcOffsetMinutes);
            var localStart = clock.ToLocal(block.StartedAt);
            var day = localStart.Date;
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);

            var member = AttendanceOf(perMember, block.UserId);
            member.Totals.Add(minutes, block.Productivity, IdleMinutesOf(block.IdleSeconds, minutes));
            member.MarkPresent(
                day,
                startMinutes: (localStart - day).TotalMinutes,
                endMinutes: (clock.ToLocal(block.EndedAt) - day).TotalMinutes);
        }

        var users = await UsersAsync(roster.Keys.Concat(perMember.Keys));

        // Absent members are listed too: everyone expected to track at some point of the range, as long as their
        // account is still active.
        foreach (var (userId, windows) in roster)
        {
            if (windows.Any(window => window.Overlaps(scope.From, scope.To)) &&
                users.TryGetValue(userId, out var user) && user.IsActive && !user.IsDeleted)
            {
                AttendanceOf(perMember, userId);
            }
        }

        var weekdays = calendar.DaysOverlapping(scope.From, scope.To).Where(IsWeekday).ToList();

        // A weekday can be missed only when it lies wholly inside the range, is not after today (in the report's
        // zone), and — per member — lies wholly inside one of their membership windows (after they joined, before the
        // project was archived).
        var today = calendar.ZoneClock.ToLocal(Clock.Now).Date;
        var assessable = weekdays
            .Where(day => day <= today)
            .Select(day => (Day: day, StartUtc: calendar.StartOfDayUtc(day), EndUtc: calendar.StartOfDayUtc(day.AddDays(1))))
            .Where(day => day.StartUtc >= scope.From && day.EndUtc <= scope.To)
            .ToList();

        var rows = perMember
            .Select(entry =>
            {
                var member = entry.Value;
                // Absence is only assessed for members: someone who left (or became a client) is not expected.
                var windows = roster.GetValueOrDefault(entry.Key);
                return new AttendanceRowDto
                {
                    UserId = entry.Key,
                    UserName = UserNameOf(users, entry.Key),
                    DaysPresent = weekdays.Count(member.Days.ContainsKey),
                    DaysAbsent = windows == null
                        ? 0
                        : assessable.Count(day =>
                            !member.Days.ContainsKey(day.Day) &&
                            windows.Any(window => window.Covers(day.StartUtc, day.EndUtc))),
                    WeekendDaysWorked = member.Days.Keys.Count(day => !IsWeekday(day)),
                    WorkedMinutes = member.Totals.RoundedMinutes,
                    IdleMinutes = member.Totals.RoundedIdleMinutes,
                    AverageStartMinutes = member.Days.Count == 0
                        ? null
                        : Math.Round(member.Days.Values.Average(d => d.FirstStartMinutes), 2),
                    AverageEndMinutes = member.Days.Count == 0
                        ? null
                        : Math.Round(member.Days.Values.Average(d => d.LastEndMinutes), 2),
                    PresentDays = member.Days.Keys.OrderBy(day => day).ToList()
                };
            })
            .OrderByDescending(r => r.WorkedMinutes)
            .ThenBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.UserId)
            .ToList();

        return new AttendanceReportDto { Days = weekdays, Rows = rows };
    }

    public async Task<List<UserDailyReportRowDto>> GetUserDailySeriesAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var calendar = ReportCalendar.For(input);
        var activities = await ScopedActivitiesAsync(scope);

        var perUserDay = new Dictionary<(Guid UserId, DateTime Day), UserDay>();
        await foreach (var block in StreamAsync(activities.Select(BlockProjection(calendar.MemberBasis))))
        {
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);
            var key = (block.UserId, DayOf(calendar, block));
            if (!perUserDay.TryGetValue(key, out var userDay))
            {
                userDay = new UserDay();
                perUserDay[key] = userDay;
            }

            userDay.Add(block.StartedAt, block.EndedAt, minutes, IdleMinutesOf(block.IdleSeconds, minutes));
        }

        var users = await UsersAsync(perUserDay.Keys.Select(k => k.UserId));

        return perUserDay
            .Select(entry => new UserDailyReportRowDto
            {
                UserId = entry.Key.UserId,
                UserName = UserNameOf(users, entry.Key.UserId),
                Date = entry.Key.Day,
                ActivityCount = entry.Value.Totals.Count,
                TrackedMinutes = entry.Value.Totals.RoundedMinutes,
                IdleMinutes = entry.Value.Totals.RoundedIdleMinutes,
                FirstStartedAt = entry.Value.FirstStartedAt,
                LastEndedAt = entry.Value.LastEndedAt
            })
            .OrderBy(r => r.Date)
            .ThenBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.UserId)
            .ToList();
    }

    public async Task<PayrollReportDto> GetPayrollAsync(GetReportSummaryInput input)
    {
        // Pay rates are Team.Manage data (as on the project roster); refused outright rather than shown as zeros.
        await AuthorizationService.CheckAsync(TrackerPermissions.Team.Manage);

        var scope = await ResolveScopeAsync(input);
        var activities = await ScopedActivitiesAsync(scope);
        var rates = await RateSchedulesAsync(activities);

        var perUserProject = new Dictionary<(Guid UserId, Guid ProjectId), BlockTotals>();
        await foreach (var block in StreamAsync(activities.Select(BlockProjection(withDeviceClock: false))))
        {
            var minutes = MinutesOf(block.StartedAt, block.EndedAt);
            var pair = TotalsOf(perUserProject, (block.UserId, block.ProjectId));
            pair.Add(minutes, block.Productivity, IdleMinutesOf(block.IdleSeconds, minutes));
            pair.AddPay(minutes, RateAt(rates, block.UserId, block.ProjectId, block.StartedAt));
        }

        var users = await UsersAsync(perUserProject.Keys.Select(k => k.UserId));
        var projects = await ProjectsAsync(perUserProject.Keys.Select(k => k.ProjectId));
        var rows = UserProjectRows(perUserProject, users, projects, withPay: true);

        return new PayrollReportDto
        {
            Rows = rows,
            TotalMinutes = Math.Round(perUserProject.Values.Sum(t => t.Minutes), 2),
            TotalAmount = rows.Sum(r => r.Amount)
        };
    }

    public async Task<List<HourlyReportRowDto>> GetHourlyAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var calendar = ReportCalendar.For(input);
        var activities = await ScopedActivitiesAsync(scope);

        var tracked = new double[7, 24];
        var active = new double[7, 24];
        await foreach (var block in StreamAsync(activities.Select(BlockProjection(calendar.MemberBasis))))
        {
            var clock = calendar.ClockFor(block.TimeZone, block.UtcOffsetMinutes);
            var activeShare = block.Productivity.HasValue ? Math.Clamp(block.Productivity.Value, 0, 100) / 100.0 : 0;

            // Walk the block one local clock hour at a time; each piece counts in the hour it falls in. The next
            // boundary is computed from the local time, so half-hour zones (Asia/Kathmandu is +05:45) and DST
            // transitions (an hour that is skipped or repeated) land on the right hour.
            var at = ReportCalendar.AsUtc(block.StartedAt);
            var end = ReportCalendar.AsUtc(block.EndedAt);
            while (at < end)
            {
                var local = clock.ToLocal(at);
                var intoHour = local.TimeOfDay - TimeSpan.FromHours(local.Hour);
                var next = at + (TimeSpan.FromHours(1) - intoHour);
                if (next > end)
                {
                    next = end;
                }

                var minutes = (next - at).TotalMinutes;
                tracked[(int)local.DayOfWeek, local.Hour] += minutes;
                active[(int)local.DayOfWeek, local.Hour] += minutes * activeShare;
                at = next;
            }
        }

        var rows = new List<HourlyReportRowDto>(7 * 24);
        for (var dayOfWeek = 0; dayOfWeek < 7; dayOfWeek++)
        {
            for (var hour = 0; hour < 24; hour++)
            {
                rows.Add(new HourlyReportRowDto
                {
                    DayOfWeek = dayOfWeek,
                    Hour = hour,
                    TrackedMinutes = Math.Round(tracked[dayOfWeek, hour], 2),
                    ActiveMinutes = Math.Round(active[dayOfWeek, hour], 2)
                });
            }
        }

        return rows;
    }

    public async Task<List<DeviceReportRowDto>> GetDevicesAsync(GetReportSummaryInput input)
    {
        var scope = await ResolveScopeAsync(input);
        var activities = await ScopedActivitiesAsync(scope);

        var rows = activities.Select(a => new DeviceBlock
        {
            UserId = a.UserId,
            DeviceId = a.DeviceId,
            DeviceName = a.DeviceName,
            Platform = a.Platform,
            OsVersion = a.OsVersion,
            AgentVersion = a.AgentVersion,
            StartedAt = a.StartedAt,
            EndedAt = a.EndedAt
        });

        var devices = new Dictionary<(Guid UserId, Guid? DeviceId), DeviceUsage>();
        await foreach (var block in StreamAsync(rows))
        {
            var key = (block.UserId, block.DeviceId);
            if (!devices.TryGetValue(key, out var usage))
            {
                usage = new DeviceUsage(block);
                devices[key] = usage;
            }

            usage.Add(block);
        }

        var users = await UsersAsync(devices.Keys.Select(k => k.UserId));

        return devices.Values
            .Select(usage => new DeviceReportRowDto
            {
                UserId = usage.Latest.UserId,
                UserName = UserNameOf(users, usage.Latest.UserId),
                DeviceId = usage.Latest.DeviceId,
                DeviceName = usage.Latest.DeviceName,
                Platform = usage.Latest.Platform,
                OsVersion = usage.Latest.OsVersion,
                AgentVersion = usage.Latest.AgentVersion,
                LastActivityAt = ReportCalendar.AsUtc(usage.LastEndedAt),
                TrackedMinutes = Math.Round(usage.Minutes, 2)
            })
            .OrderBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.UserId)
            .ThenByDescending(r => r.LastActivityAt)
            .ThenBy(r => r.DeviceId)
            .ToList();
    }

    // ---- Scope ----

    /// <summary>Who and what a report request may see; see the class remarks for the rules.</summary>
    private sealed class ReportScope
    {
        public Guid CallerId { get; init; }

        /// <summary>Every member's data (ViewAll + Reporting).</summary>
        public bool IsTeam { get; init; }

        /// <summary>For non-team callers: projects whose every member's data they may see (they are a client there).</summary>
        public List<Guid> ClientProjectIds { get; init; } = new();

        /// <summary>The member to report on (null: everyone in scope).</summary>
        public Guid? UserId { get; init; }

        public Guid? ProjectId { get; init; }

        public DateTime From { get; init; }
        public DateTime To { get; init; }
    }

    private async Task<ReportScope> ResolveScopeAsync(GetReportSummaryInput input)
    {
        var from = ReportCalendar.AsUtc(input.From);
        var to = ReportCalendar.AsUtc(input.To);
        EnsureValidRange(from, to);

        var callerId = CurrentUser.GetId();

        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Reporting.Default))
            {
                return new ReportScope
                {
                    CallerId = callerId,
                    IsTeam = true,
                    UserId = input.UserId,
                    ProjectId = input.ProjectId,
                    From = from,
                    To = to
                };
            }

            // Team reports are what this caller expects, and what they may not see: refuse instead of answering with
            // their own figures under the team's label. Their own reports stay open to them.
            if (input.UserId != callerId)
            {
                throw new AbpAuthorizationException(
                    $"Team reports require the {TrackerPermissions.Reporting.Default} permission.");
            }

            return new ReportScope { CallerId = callerId, UserId = callerId, ProjectId = input.ProjectId, From = from, To = to };
        }

        var clientProjectIds = await ClientProjectIdsAsync(callerId);
        return new ReportScope
        {
            CallerId = callerId,
            ClientProjectIds = clientProjectIds,
            // A member who only sees their own data always gets it (a UserId cannot widen the scope); a client may
            // narrow the client view down to one member.
            UserId = clientProjectIds.Count == 0 ? callerId : input.UserId,
            ProjectId = input.ProjectId,
            From = from,
            To = to
        };
    }

    /// <summary>Projects the caller is a client of.</summary>
    private async Task<List<Guid>> ClientProjectIdsAsync(Guid callerId)
    {
        var memberships = await _memberRepository.GetQueryableAsync();
        var roles = await AsyncExecuter.ToListAsync(
            memberships.Where(m => m.UserId == callerId).Select(m => new { m.ProjectId, m.Role }));

        return roles.Where(m => TrackerRoles.IsClientProjectRole(m.Role)).Select(m => m.ProjectId).Distinct().ToList();
    }

    /// <summary>The range's blocks (by start) visible in the scope, filtered by project and member.</summary>
    private async Task<IQueryable<Activity>> ScopedActivitiesAsync(ReportScope scope)
    {
        var from = scope.From;
        var to = scope.To;
        var query = (await _activityRepository.GetQueryableAsync())
            .Where(a => a.StartedAt >= from && a.StartedAt < to);

        if (scope.ProjectId.HasValue)
        {
            var projectId = scope.ProjectId.Value;
            query = query.Where(a => a.ProjectId == projectId);
        }

        if (!scope.IsTeam)
        {
            var callerId = scope.CallerId;
            var clientProjectIds = scope.ClientProjectIds;
            query = clientProjectIds.Count == 0
                ? query.Where(a => a.UserId == callerId)
                : query.Where(a => a.UserId == callerId || clientProjectIds.Contains(a.ProjectId));
        }

        if (scope.UserId.HasValue)
        {
            var userId = scope.UserId.Value;
            query = query.Where(a => a.UserId == userId);
        }

        return query;
    }

    /// <summary>A span (UTC) during which a member was expected to track on a reported project: from joining it until it
    /// was archived (open-ended while the project is active).</summary>
    private readonly record struct MembershipWindow(DateTime From, DateTime? Until)
    {
        /// <summary>True when the member was expected at some point of [<paramref name="from"/>, <paramref name="to"/>).</summary>
        public bool Overlaps(DateTime from, DateTime to) => From < to && (Until == null || Until.Value > from);

        /// <summary>True when the whole day [<paramref name="dayStart"/>, <paramref name="dayEnd"/>) lies in the window.</summary>
        public bool Covers(DateTime dayStart, DateTime dayEnd) => dayStart >= From && (Until == null || dayEnd <= Until.Value);
    }

    /// <summary>
    /// The attendance roster: members of the reported projects (clients excluded), each with their membership windows —
    /// the span of each membership, from joining until the project was archived. An archived project still counts for
    /// the days before its archiving; one archived before its moment was recorded expects nobody. Deleted projects
    /// take their memberships with them.
    /// </summary>
    private async Task<Dictionary<Guid, List<MembershipWindow>>> RosterAsync(ReportScope scope)
    {
        var memberships = await _memberRepository.GetQueryableAsync();
        var projects = await _projectRepository.GetQueryableAsync();

        if (scope.ProjectId.HasValue)
        {
            var projectId = scope.ProjectId.Value;
            memberships = memberships.Where(m => m.ProjectId == projectId);
        }

        if (!scope.IsTeam)
        {
            var callerId = scope.CallerId;
            var clientProjectIds = scope.ClientProjectIds;
            memberships = clientProjectIds.Count == 0
                ? memberships.Where(m => m.UserId == callerId)
                : memberships.Where(m => m.UserId == callerId || clientProjectIds.Contains(m.ProjectId));
        }

        if (scope.UserId.HasValue)
        {
            var userId = scope.UserId.Value;
            memberships = memberships.Where(m => m.UserId == userId);
        }

        var rows = await AsyncExecuter.ToListAsync(
            from m in memberships
            join p in projects on m.ProjectId equals p.Id
            select new { m.UserId, m.Role, m.CreationTime, p.IsArchived, p.ArchivedAt });

        return rows
            .Where(m => !TrackerRoles.IsClientProjectRole(m.Role) && (!m.IsArchived || m.ArchivedAt.HasValue))
            .GroupBy(m => m.UserId)
            .ToDictionary(
                g => g.Key,
                g => g
                    .Select(m => new MembershipWindow(
                        ReportCalendar.AsUtc(m.CreationTime),
                        m.IsArchived ? ReportCalendar.AsUtc(m.ArchivedAt!.Value) : null))
                    .ToList());
    }

    /// <summary>To must be after From, and the span is capped so a single request cannot scan (or, for the
    /// day-by-day reports, enumerate) an unbounded history.</summary>
    private static void EnsureValidRange(DateTime from, DateTime to)
    {
        if (to <= from || (to - from).TotalDays > GetReportSummaryInput.MaxRangeDays)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid)
                .WithData("maxDays", GetReportSummaryInput.MaxRangeDays)
                .WithData("from", from)
                .WithData("to", to);
        }
    }

    // ---- Reading blocks ----

    /// <summary>The scalar slice of a block the totals-based reports read (no window JSON, no captures).</summary>
    private sealed class BlockRow
    {
        public Guid UserId { get; set; }
        public Guid ProjectId { get; set; }
        public DateTime StartedAt { get; set; }
        public DateTime EndedAt { get; set; }
        public int? Productivity { get; set; }
        public int? MouseClicks { get; set; }
        public int? KeyboardHits { get; set; }
        public int? IdleSeconds { get; set; }
        public string? TimeZone { get; set; }
        public int? UtcOffsetMinutes { get; set; }
    }

    /// <summary>The block projection; the device's clock (time zone and offset) is only read for the member basis.</summary>
    private static Expression<Func<Activity, BlockRow>> BlockProjection(bool withDeviceClock)
    {
        if (withDeviceClock)
        {
            return a => new BlockRow
            {
                UserId = a.UserId,
                ProjectId = a.ProjectId,
                StartedAt = a.StartedAt,
                EndedAt = a.EndedAt,
                Productivity = a.Productivity,
                MouseClicks = a.MouseClicks,
                KeyboardHits = a.KeyboardHits,
                IdleSeconds = a.IdleSeconds,
                TimeZone = a.TimeZone,
                UtcOffsetMinutes = a.UtcOffsetMinutes
            };
        }

        return a => new BlockRow
        {
            UserId = a.UserId,
            ProjectId = a.ProjectId,
            StartedAt = a.StartedAt,
            EndedAt = a.EndedAt,
            Productivity = a.Productivity,
            MouseClicks = a.MouseClicks,
            KeyboardHits = a.KeyboardHits,
            IdleSeconds = a.IdleSeconds
        };
    }

    /// <summary>Reads a query's rows one at a time as the provider streams them, instead of buffering the range.</summary>
    private async IAsyncEnumerable<T> StreamAsync<T>(IQueryable<T> query)
    {
        if (query is IAsyncEnumerable<T> stream)
        {
            await foreach (var row in stream)
            {
                yield return row;
            }

            yield break;
        }

        foreach (var row in await AsyncExecuter.ToListAsync(query))
        {
            yield return row;
        }
    }

    /// <summary>The local day a block belongs to: the day of its start, on the clock the calendar reads it on.</summary>
    private static DateTime DayOf(ReportCalendar calendar, BlockRow block)
    {
        return calendar.ClockFor(block.TimeZone, block.UtcOffsetMinutes).ToLocal(block.StartedAt).Date;
    }

    private static double MinutesOf(DateTime start, DateTime end) => (end - start).TotalMinutes;

    /// <summary>The block's measured idle time in minutes, never more than the block lasted; 0 when not measured.</summary>
    private static double IdleMinutesOf(int? idleSeconds, double blockMinutes)
    {
        return idleSeconds is > 0 ? Math.Min(idleSeconds.Value / 60.0, Math.Max(blockMinutes, 0)) : 0;
    }

    private static bool IsWeekday(DateTime day) => day.DayOfWeek is not (DayOfWeek.Saturday or DayOfWeek.Sunday);

    private static BlockTotals TotalsOf<TKey>(Dictionary<TKey, BlockTotals> groups, TKey key)
        where TKey : notnull
    {
        if (!groups.TryGetValue(key, out var totals))
        {
            totals = new BlockTotals();
            groups[key] = totals;
        }

        return totals;
    }

    // ---- Names, titles, rates ----

    /// <summary>The accounts behind these ids in one query, deleted ones included so past time keeps its name.</summary>
    private async Task<Dictionary<Guid, IdentityUser>> UsersAsync(IEnumerable<Guid> userIds)
    {
        var ids = userIds.Distinct().ToList();
        if (ids.Count == 0)
        {
            return new Dictionary<Guid, IdentityUser>();
        }

        using (DataFilter.Disable<ISoftDelete>())
        {
            return (await _userRepository.GetListByIdsAsync(ids)).ToDictionary(u => u.Id);
        }
    }

    private sealed record ProjectInfo(string Title, bool IsArchived);

    /// <summary>Titles of these projects in one query, archived and deleted ones included (their time still counts).</summary>
    private async Task<Dictionary<Guid, ProjectInfo>> ProjectsAsync(IEnumerable<Guid> projectIds)
    {
        var ids = projectIds.Distinct().ToList();
        if (ids.Count == 0)
        {
            return new Dictionary<Guid, ProjectInfo>();
        }

        using (DataFilter.Disable<ISoftDelete>())
        {
            var projects = await _projectRepository.GetQueryableAsync();
            var rows = await AsyncExecuter.ToListAsync(projects
                .Where(p => ids.Contains(p.Id))
                .Select(p => new { p.Id, p.Title, p.IsArchived, p.IsDeleted }));

            return rows.ToDictionary(p => p.Id, p => new ProjectInfo(p.Title, p.IsArchived || p.IsDeleted));
        }
    }

    private static string UserNameOf(Dictionary<Guid, IdentityUser> users, Guid userId)
    {
        return users.TryGetValue(userId, out var user) ? user.UserName : string.Empty;
    }

    private static string? FullNameOf(IdentityUser? user)
    {
        if (user == null)
        {
            return null;
        }

        var name = string.Join(' ', new[] { user.Name, user.Surname }
            .Where(part => !string.IsNullOrWhiteSpace(part))
            .Select(part => part!.Trim()));
        return name.Length == 0 ? null : name;
    }

    /// <summary>
    /// The rate history of every member × project with time in the scope: one query for those pairs (a DISTINCT in
    /// the database), one for their <see cref="ProjectMemberRate"/> rows, and — only when a pair has no history, as
    /// memberships recorded before rate history existed — one for the memberships, whose current rate then applies.
    /// </summary>
    private async Task<Dictionary<(Guid UserId, Guid ProjectId), HourlyRateSchedule>> RateSchedulesAsync(
        IQueryable<Activity> activities)
    {
        var pairs = await AsyncExecuter.ToListAsync(activities.Select(a => new { a.UserId, a.ProjectId }).Distinct());
        var schedules = new Dictionary<(Guid UserId, Guid ProjectId), HourlyRateSchedule>();
        if (pairs.Count == 0)
        {
            return schedules;
        }

        var userIds = pairs.Select(p => p.UserId).Distinct().ToList();
        var projectIds = pairs.Select(p => p.ProjectId).Distinct().ToList();

        var history = (await _rateRepository.GetListAsync(r => userIds.Contains(r.UserId) && projectIds.Contains(r.ProjectId)))
            .ToLookup(r => (r.UserId, r.ProjectId));

        var currentRates = new Dictionary<(Guid UserId, Guid ProjectId), decimal>();
        if (pairs.Any(p => !history.Contains((p.UserId, p.ProjectId))))
        {
            var memberships = await _memberRepository.GetListAsync(
                m => userIds.Contains(m.UserId) && projectIds.Contains(m.ProjectId));
            foreach (var membership in memberships.OrderBy(m => m.CreationTime))
            {
                currentRates[(membership.UserId, membership.ProjectId)] = membership.HourlyRate;
            }
        }

        foreach (var pair in pairs)
        {
            var key = (pair.UserId, pair.ProjectId);
            schedules[key] = history.Contains(key)
                ? HourlyRateSchedule.From(history[key])
                : currentRates.TryGetValue(key, out var rate)
                    ? HourlyRateSchedule.Flat(rate)
                    : HourlyRateSchedule.None;
        }

        return schedules;
    }

    private static decimal RateAt(
        Dictionary<(Guid UserId, Guid ProjectId), HourlyRateSchedule> rates, Guid userId, Guid projectId, DateTime startedAt)
    {
        return rates.TryGetValue((userId, projectId), out var schedule) ? schedule.RateAt(startedAt) : 0m;
    }

    private static List<UserProjectReportRowDto> UserProjectRows(
        Dictionary<(Guid UserId, Guid ProjectId), BlockTotals> perUserProject,
        Dictionary<Guid, IdentityUser> users,
        Dictionary<Guid, ProjectInfo> projects,
        bool withPay)
    {
        return perUserProject
            .Select(entry => new UserProjectReportRowDto
            {
                UserId = entry.Key.UserId,
                UserName = UserNameOf(users, entry.Key.UserId),
                ProjectId = entry.Key.ProjectId,
                ProjectTitle = projects.GetValueOrDefault(entry.Key.ProjectId)?.Title ?? string.Empty,
                ActivityCount = entry.Value.Count,
                TrackedMinutes = entry.Value.RoundedMinutes,
                IdleMinutes = entry.Value.RoundedIdleMinutes,
                AverageProductivity = entry.Value.AverageProductivity,
                Amount = withPay ? entry.Value.Amount : 0m,
                HourlyRate = withPay ? entry.Value.HourlyRate : 0m
            })
            .OrderBy(r => r.UserName, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.UserId)
            .ThenBy(r => r.ProjectTitle, StringComparer.OrdinalIgnoreCase)
            .ThenBy(r => r.ProjectId)
            .ToList();
    }

    // ---- Attendance, per-member days and devices ----

    /// <summary>One member's presence: their totals and, per local day, the first start and last end.</summary>
    private sealed class MemberAttendance
    {
        public BlockTotals Totals { get; } = new();
        public Dictionary<DateTime, PresentDay> Days { get; } = new();

        public void MarkPresent(DateTime day, double startMinutes, double endMinutes)
        {
            if (Days.TryGetValue(day, out var present))
            {
                present.FirstStartMinutes = Math.Min(present.FirstStartMinutes, startMinutes);
                present.LastEndMinutes = Math.Max(present.LastEndMinutes, endMinutes);
            }
            else
            {
                Days[day] = new PresentDay { FirstStartMinutes = startMinutes, LastEndMinutes = endMinutes };
            }
        }
    }

    /// <summary>Minutes after the day's local midnight of its first start and last end (past 1440 after midnight).</summary>
    private sealed class PresentDay
    {
        public double FirstStartMinutes { get; set; }
        public double LastEndMinutes { get; set; }
    }

    private static MemberAttendance AttendanceOf(Dictionary<Guid, MemberAttendance> members, Guid userId)
    {
        if (!members.TryGetValue(userId, out var member))
        {
            member = new MemberAttendance();
            members[userId] = member;
        }

        return member;
    }

    private sealed class UserDay
    {
        public BlockTotals Totals { get; } = new();
        public DateTime FirstStartedAt { get; private set; } = DateTime.MaxValue;
        public DateTime LastEndedAt { get; private set; } = DateTime.MinValue;

        public void Add(DateTime startedAt, DateTime endedAt, double minutes, double idleMinutes)
        {
            Totals.Add(minutes, productivity: null, idleMinutes);
            var start = ReportCalendar.AsUtc(startedAt);
            var end = ReportCalendar.AsUtc(endedAt);
            FirstStartedAt = start < FirstStartedAt ? start : FirstStartedAt;
            LastEndedAt = end > LastEndedAt ? end : LastEndedAt;
        }
    }

    private sealed class DeviceBlock
    {
        public Guid UserId { get; set; }
        public Guid? DeviceId { get; set; }
        public string? DeviceName { get; set; }
        public string? Platform { get; set; }
        public string? OsVersion { get; set; }
        public string? AgentVersion { get; set; }
        public DateTime StartedAt { get; set; }
        public DateTime EndedAt { get; set; }
    }

    /// <summary>One member's use of one device: total minutes, and the latest block (whose details describe it).</summary>
    private sealed class DeviceUsage
    {
        public DeviceUsage(DeviceBlock first)
        {
            Latest = first;
            LastEndedAt = first.EndedAt;
        }

        public DeviceBlock Latest { get; private set; }
        public DateTime LastEndedAt { get; private set; }
        public double Minutes { get; private set; }

        public void Add(DeviceBlock block)
        {
            Minutes += MinutesOf(block.StartedAt, block.EndedAt);
            LastEndedAt = block.EndedAt > LastEndedAt ? block.EndedAt : LastEndedAt;
            if (block.StartedAt > Latest.StartedAt ||
                (block.StartedAt == Latest.StartedAt && block.EndedAt > Latest.EndedAt))
            {
                Latest = block;
            }
        }
    }

    // ---- App usage ----

    private sealed class AppUsageBlock
    {
        public Guid UserId { get; set; }
        public DateTime StartedAt { get; set; }
        public DateTime EndedAt { get; set; }
        public int? IdleSeconds { get; set; }
        public string ActiveWindowsJson { get; set; } = "[]";
        public string RunningProgramsJson { get; set; } = "[]";

        /// <summary>Only read for blocks with idle time.</summary>
        public string? TimelineJson { get; set; }
    }

    /// <summary>An app named in a block: the spelling the agent sent, and its canonical group key.</summary>
    private readonly record struct AppEntry(string Raw, string Key, int? Seconds);

    /// <summary>One canonical app's totals; tracks every spelling's credited minutes to name the row after the main one.</summary>
    private sealed class AppUsageTotals
    {
        private readonly Dictionary<string, (double Minutes, int Seen)> _spellings = new(StringComparer.Ordinal);

        public double Minutes { get; private set; }
        public double IdleMinutes { get; set; }
        public int ActivityCount { get; set; }
        public HashSet<Guid> Users { get; } = new();

        public void Credit(string spelling, double minutes)
        {
            Minutes += minutes;
            var (credited, seen) = _spellings.GetValueOrDefault(spelling);
            _spellings[spelling] = (credited + minutes, seen + 1);
        }

        /// <summary>The spelling with the most credited minutes (then the most sightings, then ordinal order).</summary>
        public string MostCreditedSpelling()
        {
            return _spellings
                .OrderByDescending(s => s.Value.Minutes)
                .ThenByDescending(s => s.Value.Seen)
                .ThenBy(s => s.Key, StringComparer.Ordinal)
                .First()
                .Key;
        }
    }

    private static AppUsageTotals AppTotalsOf(Dictionary<string, AppUsageTotals> apps, string key)
    {
        if (!apps.TryGetValue(key, out var app))
        {
            app = new AppUsageTotals();
            apps[key] = app;
        }

        return app;
    }

    /// <summary>
    /// The block's minutes per app (by canonical key, then by spelling). Newer agents report per-window foreground
    /// seconds: each app is credited with its own share of the block, and over-reported totals are scaled down so a
    /// block is never credited more minutes than it lasted. Legacy blocks only carry the window focused at capture
    /// time: that app (the first active window) is credited with the whole block; other apps present in the block
    /// count towards their reach but not their minutes (avoids double-counting).
    /// </summary>
    private static Dictionary<string, Dictionary<string, double>> CreditFocusedTime(List<AppEntry> active, double blockMinutes)
    {
        var credit = new Dictionary<string, Dictionary<string, double>>(StringComparer.Ordinal);

        void Add(AppEntry entry, double minutes)
        {
            if (!credit.TryGetValue(entry.Key, out var spellings))
            {
                spellings = new Dictionary<string, double>(StringComparer.Ordinal);
                credit[entry.Key] = spellings;
            }

            spellings[entry.Raw] = spellings.GetValueOrDefault(entry.Raw) + minutes;
        }

        var timed = active.Where(e => e.Seconds > 0).ToList();
        if (timed.Count > 0)
        {
            var reportedMinutes = timed.Sum(e => e.Seconds!.Value / 60.0);
            var scale = reportedMinutes > blockMinutes ? blockMinutes / reportedMinutes : 1;
            foreach (var entry in timed)
            {
                Add(entry, entry.Seconds!.Value / 60.0 * scale);
            }
        }
        else if (active.Count > 0)
        {
            Add(active[0], blockMinutes);
        }

        return credit;
    }

    /// <summary>
    /// The block's idle minutes per credited app (by canonical key). Idle minutes the timeline places in an app's
    /// foreground go to that app; the rest of the block's idle time is shared by its apps in proportion to their
    /// credited minutes. No app is given more idle time than it was credited.
    /// </summary>
    private static Dictionary<string, double> IdleByApp(
        double idleMinutes, string? timelineJson, double blockMinutes, Dictionary<string, Dictionary<string, double>> credit)
    {
        var result = new Dictionary<string, double>(StringComparer.Ordinal);
        var creditedMinutes = credit.ToDictionary(c => c.Key, c => c.Value.Values.Sum(), StringComparer.Ordinal);
        var creditTotal = creditedMinutes.Values.Sum();
        if (idleMinutes <= 0 || creditTotal <= 0)
        {
            return result;
        }

        var placed = new Dictionary<string, double>(StringComparer.Ordinal);
        foreach (var (key, minutes) in IdleTimelineMinutes(timelineJson, blockMinutes))
        {
            if (creditedMinutes.ContainsKey(key))
            {
                placed[key] = placed.GetValueOrDefault(key) + minutes;
            }
        }

        var placedTotal = placed.Values.Sum();
        var placedScale = placedTotal > idleMinutes ? idleMinutes / placedTotal : 1;
        var rest = Math.Max(0, idleMinutes - placedTotal * placedScale);

        foreach (var (key, minutes) in creditedMinutes)
        {
            var idle = placed.GetValueOrDefault(key) * placedScale + rest * minutes / creditTotal;
            result[key] = Math.Min(idle, minutes);
        }

        return result;
    }

    /// <summary>
    /// The idle minute buckets of a stored timeline, as (canonical app key, minutes) — buckets without an app name
    /// are skipped. Buckets follow the agents' rule: the block length rounded to the nearest minute (at least one)
    /// gives the bucket count, every bucket lasts 60 s and the last one absorbs the remainder.
    /// </summary>
    private static IEnumerable<(string Key, double Minutes)> IdleTimelineMinutes(string? timelineJson, double blockMinutes)
    {
        if (string.IsNullOrWhiteSpace(timelineJson) || blockMinutes <= 0)
        {
            yield break;
        }

        List<ActivityMinuteDto>? timeline;
        try
        {
            timeline = JsonSerializer.Deserialize<List<ActivityMinuteDto>>(timelineJson, StoredJsonOptions);
        }
        catch (JsonException)
        {
            yield break;
        }

        if (timeline == null)
        {
            yield break;
        }

        var buckets = Math.Max(1, (int)Math.Round(blockMinutes, MidpointRounding.AwayFromZero));
        var lastBucketMinutes = blockMinutes - (buckets - 1);
        var seen = new HashSet<int>();
        foreach (var minute in timeline)
        {
            if (minute == null || minute.Active || minute.Minute < 0 || minute.Minute >= buckets || !seen.Add(minute.Minute))
            {
                continue;
            }

            var key = AppNames.GroupKey(minute.AppName);
            if (key.Length > 0)
            {
                yield return (key, minute.Minute == buckets - 1 ? lastBucketMinutes : 1);
            }
        }
    }

    /// <summary>The entries of a stored window-info JSON array that name an application (in stored order, with
    /// per-window seconds when the agent reported them); tolerant of bad data.</summary>
    private static List<AppEntry> ParseApps(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return new List<AppEntry>();
        }

        List<WindowInfoDto>? windows;
        try
        {
            windows = JsonSerializer.Deserialize<List<WindowInfoDto>>(json, StoredJsonOptions);
        }
        catch (JsonException)
        {
            return new List<AppEntry>();
        }

        var entries = new List<AppEntry>();
        foreach (var window in windows ?? new List<WindowInfoDto>())
        {
            var key = AppNames.GroupKey(window?.AppName);
            if (key.Length > 0)
            {
                entries.Add(new AppEntry(window!.AppName!.Trim(), key, window.Seconds));
            }
        }

        return entries;
    }
}
