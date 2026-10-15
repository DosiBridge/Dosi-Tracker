using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Timing;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// Seeding helpers shared by the reporting tests. Blocks, memberships and rate history are inserted directly
/// (not through the upload endpoint), so the reports are tested against exactly the stored rows they read,
/// independent of the ingest rules.
/// </summary>
public abstract class ReportingTestBase<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    protected static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    protected static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000002");

    /// <summary>Monday 2026-07-20, 00:00 UTC.</summary>
    protected static readonly DateTime Day = new(2026, 7, 20, 0, 0, 0, DateTimeKind.Utc);

    /// <summary>Memberships are backdated to this moment unless a test says otherwise (attendance counts absences
    /// only after a member joined).</summary>
    protected static readonly DateTime LongAgo = new(2026, 1, 1, 0, 0, 0, DateTimeKind.Utc);

    protected IReportingAppService Reporting { get; }
    protected IClock TestClock { get; }

    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected ReportingTestBase()
    {
        Reporting = GetRequiredService<IReportingAppService>();
        TestClock = GetRequiredService<IClock>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _rateRepository = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected static GetReportSummaryInput Range(DateTime from, DateTime to, string? timeZone = null, string? dayBasis = null)
    {
        return new GetReportSummaryInput { From = from, To = to, TimeZone = timeZone, DayBasis = dayBasis };
    }

    /// <summary>A project (archived — at <paramref name="archivedAt"/>, <see cref="LongAgo"/> by default — or soft-deleted
    /// on request).</summary>
    protected async Task<Guid> SeedProjectAsync(
        string title = "Seeded Project", bool archived = false, bool deleted = false, DateTime? archivedAt = null)
    {
        var projectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            var project = new Project(projectId, TrackerTestConsts.WorkspaceId, title, intervalMinutes: 10);
            if (archived || archivedAt.HasValue)
            {
                project.Archive(archivedAt ?? LongAgo);
            }

            await _projectRepository.InsertAsync(project, autoSave: true);
            if (deleted)
            {
                await _projectRepository.DeleteAsync(project, autoSave: true);
            }
        });

        return projectId;
    }

    /// <summary>A project membership, joined at <paramref name="joinedAt"/> (<see cref="LongAgo"/> by default).</summary>
    protected Task AddMemberAsync(Guid projectId, Guid userId, string role = "Worker", decimal hourlyRate = 0m, DateTime? joinedAt = null)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var member = new ProjectMember(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, userId, role, hourlyRate);
            ObjectHelper.TrySetProperty(member, m => m.CreationTime, () => joinedAt ?? LongAgo);
            await _memberRepository.InsertAsync(member);
        });
    }

    /// <summary>A project and the current user's Worker membership on it.</summary>
    protected async Task<Guid> SeedMyProjectAsync(string title = "Seeded Project")
    {
        var projectId = await SeedProjectAsync(title);
        await AddMemberAsync(projectId, CurrentUserId);
        return projectId;
    }

    /// <summary>One entry of a member's rate history.</summary>
    protected Task AddRateAsync(Guid projectId, Guid userId, decimal hourlyRate, DateTime effectiveFrom)
    {
        return WithUnitOfWorkAsync(() => _rateRepository.InsertAsync(
            new ProjectMemberRate(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, userId, hourlyRate, effectiveFrom)));
    }

    /// <summary>An Identity account (active unless told otherwise).</summary>
    protected Task CreateUserAsync(Guid userId, string userName, string? name = null, string? surname = null, bool isActive = true)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(userId, userName, $"{userName}@acme.test")
            {
                Name = name,
                Surname = surname
            };
            user.SetIsActive(isActive);
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
        });
    }

    /// <summary>Deletes an Identity account (a soft delete, as in the product).</summary>
    protected Task DeleteUserAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            (await users.DeleteAsync(await users.GetByIdAsync(userId))).Succeeded.ShouldBeTrue();
        });
    }

    /// <summary>A block of <paramref name="minutes"/> starting at <paramref name="start"/> (the current user's unless
    /// <paramref name="userId"/> is given); <paramref name="configure"/> sets anything else.</summary>
    protected Activity Block(
        Guid projectId, DateTime start, double minutes, Guid? userId = null, int? productivity = 80, Action<Activity>? configure = null)
    {
        var block = new Activity(
            _guidGenerator.Create(), TrackerTestConsts.WorkspaceId, userId ?? CurrentUserId, projectId, Guid.NewGuid(), start, start.AddMinutes(minutes))
        {
            Productivity = productivity,
            MouseClicks = 10,
            KeyboardHits = 20
        };
        configure?.Invoke(block);
        return block;
    }

    protected Task InsertAsync(params Activity[] blocks)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            foreach (var block in blocks)
            {
                await _activityRepository.InsertAsync(block);
            }
        });
    }

    /// <summary>Window info as the upload endpoint stores it (absent usage fields omitted).</summary>
    protected static string StoredWindows(params (string App, int? Seconds)[] windows)
    {
        return JsonSerializer.Serialize(windows
            .Select(w => new WindowInfoDto { AppName = w.App, WindowTitle = w.App, Seconds = w.Seconds })
            .ToList());
    }

    /// <summary>A per-minute timeline as the upload endpoint stores it: minute i is idle when listed in
    /// <paramref name="idleMinutes"/>, and names the app in front.</summary>
    protected static string StoredTimeline(IReadOnlyList<string> appPerMinute, params int[] idleMinutes)
    {
        return JsonSerializer.Serialize(appPerMinute
            .Select((app, minute) => new ActivityMinuteDto
            {
                Minute = minute,
                Active = !idleMinutes.Contains(minute),
                AppName = app
            })
            .ToList());
    }
}
