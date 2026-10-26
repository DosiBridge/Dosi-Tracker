using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>
/// A small workspace for the read-layer tests: the caller and Ayesha Rahman, two projects (Website, where both work,
/// and Client portal, where the caller's role is <see cref="CallerPortalRole"/>), one day of blocks that carry every
/// kind of data AI assist must never read (a device name, an OS version, input counts, a window title, a note), and a
/// task.
/// </summary>
public abstract class AiReadLayerTestBase<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    /// <summary>Two days ago, 00:00 UTC: inside the note edit window.</summary>
    protected static readonly DateTime Day = DateTime.SpecifyKind(DateTime.UtcNow.Date.AddDays(-2), DateTimeKind.Utc);

    protected static readonly Guid AyeshaId = Guid.Parse("a7e5a000-0000-4000-8000-0000000000a1");

    protected const string AyeshaName = "Ayesha Rahman";
    protected const string SecretDeviceName = "AYESHA-LAPTOP";
    protected const string SecretOsVersion = "Windows 11 Pro 24H2";
    protected const int SecretKeystrokes = 98765;
    protected const int SecretClicks = 43210;
    protected const string SalaryTitle = "Q3 salary review.xlsx";
    protected const string SecretNote = "Prepared the board deck";

    protected Guid WebsiteId { get; private set; }
    protected Guid PortalId { get; private set; }
    protected Guid CallerBlockId { get; private set; }
    protected Guid AyeshaBlockId { get; private set; }

    /// <summary>The caller's workspace role.</summary>
    protected virtual string CallerRole => WorkspaceRoles.Owner;

    /// <summary>The caller's project role on the Client portal.</summary>
    protected virtual string CallerPortalRole => "Worker";

    protected IAiReadLayer Reads => GetRequiredService<IAiReadLayer>();

    protected IReportingAppService Reporting => GetRequiredService<IReportingAppService>();

    protected AiQuery DayQuery => new(Day, Day.AddDays(1), TimeZone: "UTC");

    protected async Task SeedAsync()
    {
        await SignInAsAsync(CallerRole);
        await CreateMemberAsync("ayesha", WorkspaceRoles.Member, AyeshaId, AyeshaName);

        WebsiteId = await SeedProjectAsync("Website", (CallerId, "Worker"), (AyeshaId, "Worker"));
        PortalId = await SeedProjectAsync("Client portal", (CallerId, CallerPortalRole), (AyeshaId, "Worker"));

        CallerBlockId = Guid.NewGuid();
        AyeshaBlockId = Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            var activities = GetRequiredService<IRepository<Activity, Guid>>();
            await activities.InsertAsync(Block(CallerBlockId, CallerId, WebsiteId, Day.AddHours(9), 30), autoSave: true);
            await activities.InsertAsync(Block(Guid.NewGuid(), CallerId, PortalId, Day.AddHours(13), 20), autoSave: true);
            await activities.InsertAsync(Block(AyeshaBlockId, AyeshaId, PortalId, Day.AddHours(10), 60), autoSave: true);
            await activities.InsertAsync(Block(Guid.NewGuid(), AyeshaId, WebsiteId, Day.AddHours(12), 45), autoSave: true);

            await GetRequiredService<IRepository<ProjectTask, Guid>>()
                .InsertAsync(new ProjectTask(Guid.NewGuid(), WorkspaceId, WebsiteId, "Ship the pricing page", "Secret task description"), autoSave: true);
        });
    }

    protected Task<AiCallerContext> CallerAsync() =>
        WithUnitOfWorkAsync(() => GetRequiredService<AiCallerContextFactory>().CreateAsync("UTC"));

    protected Task<T> ReadAsync<T>(Func<Task<T>> read) => WithUnitOfWorkAsync(read);

    protected static GetReportSummaryInput Input(Guid? userId = null) =>
        new() { From = Day, To = Day.AddDays(1), TimeZone = "UTC", UserId = userId };

    /// <summary>What the caller gets from the reporting endpoint for the team, or (when the team is refused to them)
    /// for themselves.</summary>
    protected async Task<T> DirectAsync<T>(Func<GetReportSummaryInput, Task<T>> call)
    {
        try
        {
            return await ReadAsync(() => call(Input()));
        }
        catch (AbpAuthorizationException)
        {
            return await ReadAsync(() => call(Input(CallerId)));
        }
    }

    private async Task<Guid> SeedProjectAsync(string title, params (Guid UserId, string Role)[] members)
    {
        var projectId = Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>()
                .InsertAsync(new Project(projectId, WorkspaceId, title, intervalMinutes: 10), autoSave: true);
            foreach (var (userId, role) in members)
            {
                await GetRequiredService<IRepository<ProjectMember, Guid>>()
                    .InsertAsync(new ProjectMember(Guid.NewGuid(), WorkspaceId, projectId, userId, role), autoSave: true);
            }
        });
        return projectId;
    }

    private static Activity Block(Guid id, Guid userId, Guid projectId, DateTime start, int minutes) =>
        new(id, WorkspaceId, userId, projectId, Guid.NewGuid(), start, start.AddMinutes(minutes))
        {
            Productivity = 70,
            KeyboardHits = SecretKeystrokes,
            MouseClicks = SecretClicks,
            IdleSeconds = 60,
            EndReason = "interval",
            ClockSkewSeconds = 12,
            InjectedInputEvents = 0,
            Description = SecretNote,
            Platform = "windows",
            AgentVersion = "1.4.0",
            OsVersion = SecretOsVersion,
            DeviceId = Guid.NewGuid(),
            DeviceName = SecretDeviceName,
            ActiveWindowsJson = JsonSerializer.Serialize(new[]
            {
                new WindowInfoDto { AppName = "EXCEL.EXE", WindowTitle = SalaryTitle, Seconds = minutes * 40 },
                new WindowInfoDto { AppName = "chrome.exe", WindowTitle = "Inbox", Seconds = minutes * 20 }
            }),
            TimelineJson = "[{\"Minute\":0,\"KeyboardHits\":" + SecretKeystrokes + ",\"Active\":true}]"
        };
}

/// <summary>
/// The read layer as one kind of caller: every read is at most what the same caller gets from the app service directly,
/// a caller who sees only themselves gets only themselves, and asking about someone else is refused, never narrowed.
/// </summary>
public abstract class AiReadLayerScopeTests<TStartupModule> : AiReadLayerTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    /// <summary>Whether this caller may read the team.</summary>
    protected abstract bool SeesTeam { get; }

    /// <summary>Whether this caller may read another member's blocks (Tracker.Activities.ViewAll), which is what a note
    /// suggestion for someone else's block needs.</summary>
    protected virtual bool ReadsOthersBlocks => SeesTeam;

    [Fact]
    public async Task The_Caller_Context_Should_Match_The_Variant()
    {
        await SeedAsync();

        (await CallerAsync()).CanSeeTeam.ShouldBe(SeesTeam);
    }

    [Fact]
    public async Task Every_Report_Read_Should_Be_A_Subset_Of_The_Callers_Own_Report()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        var summary = await ReadAsync(() => Reads.SummaryAsync(caller, DayQuery));
        var directSummary = await DirectAsync(Reporting.GetSummaryAsync);
        summary.Members.Select(m => m.UserId).ShouldBeSubsetOf(directSummary.PerUser.Select(u => u.UserId));
        summary.TrackedMinutes.ShouldBeLessThanOrEqualTo(directSummary.TotalTrackedMinutes);
        summary.Projects.Select(p => p.ProjectId).ShouldBeSubsetOf(directSummary.PerProject.Select(p => p.ProjectId));

        var daily = await ReadAsync(() => Reads.DailyAsync(caller, DayQuery));
        var directDaily = await DirectAsync(Reporting.GetDailySeriesAsync);
        daily.Sum(d => d.TrackedMinutes).ShouldBeLessThanOrEqualTo(directDaily.Sum(d => d.TrackedMinutes));

        var userDaily = await ReadAsync(() => Reads.UserDailyAsync(caller, DayQuery));
        userDaily.Select(d => d.UserId).ShouldBeSubsetOf((await DirectAsync(Reporting.GetUserDailySeriesAsync)).Select(d => d.UserId));

        var apps = await ReadAsync(() => Reads.AppUsageAsync(caller, DayQuery));
        apps.Sum(a => a.TrackedMinutes).ShouldBeLessThanOrEqualTo((await DirectAsync(Reporting.GetAppUsageAsync)).Sum(a => a.TrackedMinutes));

        var attendance = await ReadAsync(() => Reads.AttendanceAsync(caller, DayQuery));
        attendance.Rows.Select(r => r.UserId).ShouldBeSubsetOf((await DirectAsync(Reporting.GetAttendanceAsync)).Rows.Select(r => r.UserId));

        var hourly = await ReadAsync(() => Reads.HourlyAsync(caller, DayQuery));
        hourly.Sum(h => h.TrackedMinutes).ShouldBeLessThanOrEqualTo((await DirectAsync(Reporting.GetHourlyAsync)).Sum(h => h.TrackedMinutes));

        var devices = await ReadAsync(() => Reads.DevicesAsync(caller, DayQuery));
        devices.Select(d => d.DeviceId).ShouldBeSubsetOf((await DirectAsync(Reporting.GetDevicesAsync)).Select(d => d.DeviceId));

        var quality = await ReadAsync(() => Reads.DataQualityAsync(caller, DayQuery));
        quality.Select(q => q.UserId).ShouldBeSubsetOf((await DirectAsync(Reporting.GetDataQualityAsync)).Select(q => q.UserId));

        if (SeesTeam)
        {
            summary.TrackedMinutes.ShouldBe(directSummary.TotalTrackedMinutes);
            summary.Members.Select(m => m.UserId).ShouldBe(new[] { CallerId, AyeshaId }, ignoreOrder: true);
            summary.Members.Single(m => m.UserId == AyeshaId).DisplayName.ShouldBe(AyeshaName);
            devices.Count.ShouldBe(4);
        }
        else
        {
            summary.TrackedMinutes.ShouldBe(50);
            summary.Members.ShouldAllBe(m => m.UserId == CallerId);
            userDaily.ShouldAllBe(d => d.UserId == CallerId);
            attendance.Rows.ShouldAllBe(r => r.UserId == CallerId);
            devices.ShouldAllBe(d => d.UserId == CallerId);
            quality.ShouldAllBe(q => q.UserId == CallerId);
            daily.Sum(d => d.TrackedMinutes).ShouldBe(50);
            apps.Sum(a => a.TrackedMinutes).ShouldBe(50);
            hourly.Sum(h => h.TrackedMinutes).ShouldBe(50);
        }
    }

    [Fact]
    public async Task Day_Blocks_Should_Be_A_Subset_Of_The_Callers_Activity_List()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        var blocks = await ReadAsync(() => Reads.DayBlocksAsync(caller, null, Day, Day.AddDays(1)));
        var direct = await ReadAsync(() => GetRequiredService<IActivityAppService>()
            .GetListAsync(new GetActivitiesInput { From = Day, To = Day.AddDays(1), MaxResultCount = 1000 }));

        blocks.Select(b => b.Id).ShouldBeSubsetOf(direct.Items.Select(a => a.Id));
        blocks.ShouldAllBe(b => b.UserId == CallerId);
        blocks.Count.ShouldBe(2);
        blocks.Select(b => b.StartedAt).ShouldBe(new[] { Day.AddHours(9), Day.AddHours(13) });
        blocks[0].Apps.Select(a => a.App).ShouldBe(new[] { "Excel", "Google Chrome" });
    }

    [Fact]
    public async Task The_Roster_Projects_And_Tasks_Should_Be_Subsets_Of_What_The_Caller_Gets()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        var roster = await ReadAsync(() => Reads.RosterAsync(caller));
        var members = await ReadAsync(() => GetRequiredService<ITeamAppService>().GetMembersAsync());
        roster.Select(r => r.UserId).ShouldBeSubsetOf(members.Items.Select(m => m.UserId));
        roster.Select(r => r.UserId).ShouldBe(SeesTeam ? new[] { CallerId, AyeshaId } : new[] { CallerId }, ignoreOrder: true);

        var projects = await ReadAsync(() => Reads.ProjectsAsync(caller, includeArchived: true));
        var directProjects = await ReadAsync(() => GetRequiredService<IProjectAppService>()
            .GetListAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 1000 }));
        projects.Select(p => p.Id).ShouldBeSubsetOf(directProjects.Items.Select(p => p.Id));
        projects.Select(p => p.Id).ShouldContain(WebsiteId);

        (await ReadAsync(() => Reads.ProjectAsync(caller, WebsiteId))).Title.ShouldBe("Website");

        var tasks = await ReadAsync(() => Reads.TasksAsync(caller, WebsiteId, isCompleted: null, max: 500));
        tasks.TotalCount.ShouldBe(1);
        tasks.Items.ShouldHaveSingleItem().Title.ShouldBe("Ship the pricing page");
    }

    [Fact]
    public async Task Asking_About_Another_Member_Should_Be_Answered_Only_For_A_Team_Caller()
    {
        await SeedAsync();
        var caller = await CallerAsync();
        var aboutAyesha = DayQuery with { UserId = AyeshaId };
        var guard = GetRequiredService<AiScopeGuard>();

        if (SeesTeam)
        {
            (await ReadAsync(() => Reads.SummaryAsync(caller, aboutAyesha))).Members.ShouldHaveSingleItem().UserId.ShouldBe(AyeshaId);
            (await ReadAsync(() => Reads.DayBlocksAsync(caller, AyeshaId, Day, Day.AddDays(1)))).ShouldAllBe(b => b.UserId == AyeshaId);
            (await ReadAsync(() => guard.EnsureMemberTargetAsync(caller, AyeshaId))).ShouldBe(AyeshaId);
            Should.NotThrow(() => guard.EnsureTeamScope(caller));
            return;
        }

        // Refused, never quietly answered with the caller's own data.
        await Should.ThrowAsync<AbpAuthorizationException>(() => ReadAsync(() => Reads.SummaryAsync(caller, aboutAyesha)));
        await Should.ThrowAsync<AbpAuthorizationException>(() => ReadAsync(() => Reads.DailyAsync(caller, aboutAyesha)));
        await Should.ThrowAsync<AbpAuthorizationException>(() => ReadAsync(() => Reads.DataQualityAsync(caller, aboutAyesha)));
        await Should.ThrowAsync<AbpAuthorizationException>(() => ReadAsync(() => Reads.DayBlocksAsync(caller, AyeshaId, Day, Day.AddDays(1))));
        var refused = await Should.ThrowAsync<AbpAuthorizationException>(() => ReadAsync(() => guard.EnsureMemberTargetAsync(caller, AyeshaId)));
        refused.Message.ShouldBe(AiScopeGuard.TeamScopeMessage);
        Should.Throw<AbpAuthorizationException>(() => guard.EnsureTeamScope(caller));

        // Themselves, named or not, is fine.
        (await ReadAsync(() => guard.EnsureMemberTargetAsync(caller, CallerId))).ShouldBe(CallerId);
        (await ReadAsync(() => guard.EnsureMemberTargetAsync(caller, null))).ShouldBe(CallerId);
        (await ReadAsync(() => Reads.SummaryAsync(caller, DayQuery with { UserId = CallerId }))).TrackedMinutes.ShouldBe(50);
    }

    [Fact]
    public async Task Another_Members_Block_Should_Only_Be_Read_For_A_Note_By_Someone_Who_May_See_It()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        (await ReadAsync(() => Reads.NoteBlockAsync(caller, CallerBlockId))).UserId.ShouldBe(CallerId);

        if (ReadsOthersBlocks)
        {
            (await ReadAsync(() => Reads.NoteBlockAsync(caller, AyeshaBlockId))).UserId.ShouldBe(AyeshaId);
        }
        else
        {
            await Should.ThrowAsync<Exception>(() => ReadAsync(() => Reads.NoteBlockAsync(caller, AyeshaBlockId)));
        }
    }
}

/// <summary>An owner (every permission): the team.</summary>
public abstract class AiReadLayerOwnerScopeTests<TStartupModule> : AiReadLayerScopeTests<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override bool SeesTeam => true;

    [Fact]
    public async Task A_Target_That_Is_Not_An_Active_Member_Should_Be_Invalid()
    {
        await SeedAsync();
        var caller = await CallerAsync();
        var guard = GetRequiredService<AiScopeGuard>();

        var stranger = await Should.ThrowAsync<BusinessException>(() => ReadAsync(() => guard.EnsureMemberTargetAsync(caller, Guid.NewGuid())));
        stranger.Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);

        var elsewhere = await ServiceProvider.CreateWorkspaceAsync("Elsewhere Co");
        var outsider = Guid.NewGuid();
        await ServiceProvider.AddWorkspaceMemberAsync(outsider, WorkspaceRoles.Member, elsewhere);
        (await Should.ThrowAsync<BusinessException>(() => ReadAsync(() => guard.EnsureMemberTargetAsync(caller, outsider))))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvalidUser);
    }

    [Fact]
    public async Task Window_Titles_And_Notes_Should_Be_Read_Only_When_The_Workspace_Allows_Them()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        var blocks = await ReadAsync(() => Reads.DayBlocksAsync(caller, AyeshaId, Day, Day.AddDays(1)));
        blocks.ShouldAllBe(b => b.WindowTitles == null && b.Note == null);
        var noteBlock = await ReadAsync(() => Reads.NoteBlockAsync(caller, AyeshaBlockId));
        noteBlock.WindowTitles.ShouldBeNull();
        noteBlock.Note.ShouldBeNull();
        JsonSerializer.Serialize(blocks).ShouldNotContain("salary");

        await SetPolicyAsync(titles: true);
        caller = await CallerAsync();
        blocks = await ReadAsync(() => Reads.DayBlocksAsync(caller, AyeshaId, Day, Day.AddDays(1)));
        blocks.ShouldAllBe(b => b.WindowTitles != null && b.Note == null);
        blocks[0].WindowTitles!.Select(t => t.Title).ShouldBe(new[] { SalaryTitle, "Inbox" });
        blocks[0].WindowTitles![0].App.ShouldBe("Excel");

        await SetPolicyAsync(titles: false, notes: true);
        caller = await CallerAsync();
        noteBlock = await ReadAsync(() => Reads.NoteBlockAsync(caller, AyeshaBlockId));
        noteBlock.WindowTitles.ShouldBeNull();
        noteBlock.Note.ShouldBe(SecretNote);
    }

    [Fact]
    public async Task No_Read_Model_Should_Carry_An_Email_A_Device_Name_An_Os_Version_Or_Input_Counts()
    {
        await SeedAsync();
        await SetPolicyAsync(titles: true, notes: true);
        var caller = await CallerAsync();

        var everything = new List<object>
        {
            await ReadAsync(() => Reads.SummaryAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.DailyAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.UserDailyAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.AppUsageAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.AttendanceAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.HourlyAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.DevicesAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.DataQualityAsync(caller, DayQuery)),
            await ReadAsync(() => Reads.DayBlocksAsync(caller, AyeshaId, Day, Day.AddDays(1))),
            await ReadAsync(() => Reads.RosterAsync(caller)),
            await ReadAsync(() => Reads.ProjectsAsync(caller, includeArchived: true)),
            await ReadAsync(() => Reads.TasksAsync(caller, WebsiteId, null, 100)),
            await ReadAsync(() => Reads.NoteBlockAsync(caller, AyeshaBlockId))
        };

        var json = JsonSerializer.Serialize(everything);
        json.ShouldNotContain("@acme.test");
        json.ShouldNotContain(SecretDeviceName);
        json.ShouldNotContain(SecretOsVersion);
        json.ShouldNotContain(SecretKeystrokes.ToString());
        json.ShouldNotContain(SecretClicks.ToString());
        json.ShouldNotContain("Secret task description");
        // Present (allowed by the policy), so the check above is not vacuous.
        json.ShouldContain(AyeshaName);
        json.ShouldContain(SalaryTitle);
        json.ShouldContain(SecretNote);

        var banned = new[] { "Email", "DeviceName", "OsVersion", "MouseClicks", "KeyboardHits", "Timeline", "Captures", "Amount", "HourlyRate" };
        foreach (var type in typeof(IAiReadLayer).Assembly.GetTypes().Where(t => t.Namespace == typeof(IAiReadLayer).Namespace && t.Name.StartsWith("Ai")))
        {
            type.GetProperties().Select(p => p.Name).ShouldNotContain(name => banned.Any(b => name.Contains(b)), type.Name);
        }
    }

    [Fact]
    public async Task Archived_Projects_Should_Be_Listed_Only_When_Asked_For()
    {
        await SeedAsync();
        await WithUnitOfWorkAsync(async () =>
        {
            var projects = GetRequiredService<IRepository<Project, Guid>>();
            var portal = await projects.GetAsync(PortalId);
            portal.Archive(Day);
            await projects.UpdateAsync(portal, autoSave: true);
        });
        var caller = await CallerAsync();

        (await ReadAsync(() => Reads.ProjectsAsync(caller, includeArchived: false))).Select(p => p.Id).ShouldBe(new[] { WebsiteId });
        var all = await ReadAsync(() => Reads.ProjectsAsync(caller, includeArchived: true));
        all.Single(p => p.Id == PortalId).IsArchived.ShouldBeTrue();
        (await ReadAsync(() => Reads.SummaryAsync(caller, DayQuery))).Projects.Single(p => p.ProjectId == PortalId).IsArchived.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Read_Must_Run_As_The_Caller_It_Was_Built_For()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        await Should.ThrowAsync<InvalidOperationException>(() =>
            ReadAsync(() => Reads.SummaryAsync(caller with { UserId = AyeshaId }, DayQuery)));
        await Should.ThrowAsync<InvalidOperationException>(() =>
            ReadAsync(() => Reads.RosterAsync(caller with { WorkspaceId = Guid.NewGuid() })));
    }

    [Fact]
    public async Task Day_Blocks_Should_Cover_At_Most_One_Local_Day()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        (await Should.ThrowAsync<BusinessException>(() => ReadAsync(() => Reads.DayBlocksAsync(caller, null, Day, Day.AddDays(2)))))
            .Code.ShouldBe(TrackerDomainErrorCodes.AiRangeTooLarge);
        (await Should.ThrowAsync<BusinessException>(() => ReadAsync(() => Reads.DayBlocksAsync(caller, null, Day, Day))))
            .Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
        (await ReadAsync(() => Reads.DayBlocksAsync(caller, null, Day, Day.AddHours(25)))).Count.ShouldBe(2);
    }

    [Fact]
    public void Ranges_Should_Be_Capped_With_Room_For_A_Dst_Change()
    {
        var guard = GetRequiredService<AiScopeGuard>();

        Should.NotThrow(() => guard.EnsureRange(Day, Day.AddDays(31).AddHours(1), 31));
        var tooLong = Should.Throw<BusinessException>(() => guard.EnsureRange(Day, Day.AddDays(32), 31));
        tooLong.Code.ShouldBe(TrackerDomainErrorCodes.AiRangeTooLarge);
        tooLong.Data["maxDays"].ShouldBe(31);
        Should.Throw<BusinessException>(() => guard.EnsureRange(Day, Day, 31)).Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
        Should.Throw<BusinessException>(() => guard.EnsureRange(Day, Day.AddDays(-1), 31)).Code.ShouldBe(TrackerDomainErrorCodes.ReportRangeInvalid);
    }

    [Fact]
    public void Only_The_Supported_Reports_Should_Be_Explained()
    {
        var guard = GetRequiredService<AiScopeGuard>();

        foreach (var slug in AiScopeGuard.ReportSlugs)
        {
            guard.EnsureReportSlug(slug).ShouldBe(slug);
        }

        guard.EnsureReportSlug(" weekly ").ShouldBe("weekly");
        Should.Throw<AbpValidationException>(() => guard.EnsureReportSlug("payroll"));
        Should.Throw<AbpValidationException>(() => guard.EnsureReportSlug("Weekly"));
        Should.Throw<AbpValidationException>(() => guard.EnsureReportSlug("unknown"));
        Should.Throw<AbpValidationException>(() => guard.EnsureReportSlug(null));
        AiScopeGuard.ReportSlugs.ShouldBeSubsetOf(Output.AiRouteIds.ReportSlugs);
    }
}

/// <summary>A manager who sees every member's activity but holds no Tracker.Reporting: only themselves.</summary>
public abstract class AiReadLayerManagerWithoutReportingScopeTests<TStartupModule> : AiReadLayerScopeTests<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override bool SeesTeam => false;
    protected override bool ReadsOthersBlocks => true;
    protected override string CallerRole => WorkspaceRoles.Manager;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Reporting.Default);
    }
}

/// <summary>A worker (no ViewAll, Reporting or Team.Manage): only themselves.</summary>
public abstract class AiReadLayerWorkerScopeTests<TStartupModule> : AiReadLayerScopeTests<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override bool SeesTeam => false;
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage);
    }
}

/// <summary>
/// A worker who is also a client of the Client portal. The reporting endpoints show a client every member's time on
/// their client project; a "self" read must not, so Ayesha's portal time never reaches an answer about the caller.
/// </summary>
public abstract class AiReadLayerClientWorkerScopeTests<TStartupModule> : AiReadLayerWorkerScopeTests<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerPortalRole => TrackerRoles.ClientProjectRole;

    [Fact]
    public async Task The_Client_Projects_Members_Should_Not_Reach_A_Self_Read()
    {
        await SeedAsync();
        var caller = await CallerAsync();

        var direct = await ReadAsync(() => Reporting.GetSummaryAsync(Input()));
        direct.PerUser.Select(u => u.UserId).ShouldContain(AyeshaId);

        var read = await ReadAsync(() => Reads.SummaryAsync(caller, DayQuery));
        read.Members.Select(m => m.UserId).ShouldNotContain(AyeshaId);
        caller.IsClientOnly.ShouldBeFalse();
        caller.HasWorkerProjectMembership.ShouldBeTrue();
    }
}
