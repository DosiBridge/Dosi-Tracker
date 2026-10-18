using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.BlobStoring;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.Activities;

public abstract class ActivityAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    /// <summary>A day inside the 45-day upload window and in the past, whenever the suite runs.</summary>
    private static readonly DateTime BaseDay = DateTime.UtcNow.Date.AddDays(-10);

    private readonly IActivityAppService _activityAppService;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly HashSet<Guid> _seededProjects = new();

    protected ActivityAppServiceTests()
    {
        _activityAppService = GetRequiredService<IActivityAppService>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    /// <summary>Submit an activity, ensuring the target project exists and the current user is
    /// a member of it first (CreateAsync now requires membership).</summary>
    private async Task<ActivityDto> SubmitAsync(CreateActivityDto input, Action<Project>? configureProject = null)
    {
        await EnsureProjectAsync(input.ProjectId, configureProject);
        return await _activityAppService.CreateAsync(input);
    }

    private async Task EnsureProjectAsync(Guid projectId, Action<Project>? configureProject = null, string role = "Worker")
    {
        if (_seededProjects.Add(projectId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var project = new Project(projectId, TrackerTestConsts.WorkspaceId, "Seeded Project");
                configureProject?.Invoke(project);
                await _projectRepository.InsertAsync(project);
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, CurrentUserId, role));
            });
        }
    }

    private static CreateActivityDto NewInput(
        Guid? clientActivityId = null,
        DateTime? startedAt = null,
        DateTime? endedAt = null)
    {
        var start = startedAt ?? BaseDay.AddHours(10);
        return new CreateActivityDto
        {
            ProjectId = Guid.NewGuid(),
            ClientActivityId = clientActivityId ?? Guid.NewGuid(),
            StartedAt = start,
            EndedAt = endedAt ?? start.AddMinutes(10),
            Productivity = 80,
            MouseClicks = 120,
            KeyboardHits = 450,
            Description = "Working on the tracker",
            ActiveWindows = new List<WindowInfoDto>
            {
                new() { AppName = "code.exe", WindowTitle = "ActivityAppService.cs" }
            },
            RunningPrograms = new List<WindowInfoDto>
            {
                new() { AppName = "code.exe", WindowTitle = "ActivityAppService.cs" },
                new() { AppName = "chrome.exe", WindowTitle = "Dosi-Tracker" }
            }
        };
    }

    /// <summary>Bytes of the given length that start like a JPEG (FF D8 FF).</summary>
    private static byte[] Jpeg(int length = 8)
    {
        var bytes = new byte[length];
        bytes[0] = 0xFF;
        bytes[1] = 0xD8;
        bytes[2] = 0xFF;
        bytes[3] = 0xE0;
        return bytes;
    }

    /// <summary>Bytes of the given length that start like a PNG (89 50 4E 47).</summary>
    private static byte[] Png(int length = 8)
    {
        var bytes = new byte[length];
        bytes[0] = 0x89;
        bytes[1] = 0x50;
        bytes[2] = 0x4E;
        bytes[3] = 0x47;
        return bytes;
    }

    private Task<List<Screenshot>> GetStoredCapturesAsync(Guid activityId)
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Screenshot, Guid>>()
            .GetListAsync(s => s.ActivityId == activityId));
    }

    private Task<Activity> GetStoredAsync(Guid activityId)
    {
        return WithUnitOfWorkAsync(() => _activityRepository.GetAsync(activityId));
    }

    [Fact]
    public async Task Should_Create_Activity_For_Current_User()
    {
        var input = NewInput();

        var result = await SubmitAsync(input);

        result.Id.ShouldNotBe(Guid.Empty);
        result.UserId.ShouldBe(CurrentUserId);
        result.ProjectId.ShouldBe(input.ProjectId);
        result.ClientActivityId.ShouldBe(input.ClientActivityId);
        result.MouseClicks.ShouldBe(120);
        result.KeyboardHits.ShouldBe(450);
        result.Productivity.ShouldBe(80);
        result.ActiveWindowsJson.ShouldContain("code.exe");
        result.RunningProgramsJson.ShouldContain("chrome.exe");
    }

    [Fact]
    public async Task Create_Should_Be_Idempotent_For_Same_ClientActivityId()
    {
        var input = NewInput();

        var first = await SubmitAsync(input);
        var second = await SubmitAsync(input);

        second.Id.ShouldBe(first.Id);

        await WithUnitOfWorkAsync(async () =>
        {
            var count = await _activityRepository.CountAsync(
                a => a.ClientActivityId == input.ClientActivityId);
            count.ShouldBe(1);
        });
    }

    [Fact]
    public async Task Create_Should_Reject_Empty_ClientActivityId()
    {
        // Regression guard for the macOS agent, which omits clientActivityId today:
        // accepting Guid.Empty would silently swallow every upload after the first.
        var input = NewInput(clientActivityId: Guid.Empty);

        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityClientIdRequired);
    }

    [Fact]
    public async Task Create_Should_Reject_EndedAt_Not_After_StartedAt()
    {
        var start = BaseDay.AddHours(10);
        var input = NewInput(startedAt: start, endedAt: start.AddMinutes(-1));

        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTimeRangeInvalid);
    }

    [Fact]
    public async Task GetList_Should_Page_And_Order_By_StartedAt_Descending()
    {
        var baseTime = BaseDay.AddHours(8);
        await SubmitAsync(NewInput(startedAt: baseTime, endedAt: baseTime.AddMinutes(10)));
        await SubmitAsync(NewInput(startedAt: baseTime.AddHours(2), endedAt: baseTime.AddHours(2).AddMinutes(10)));
        await SubmitAsync(NewInput(startedAt: baseTime.AddHours(1), endedAt: baseTime.AddHours(1).AddMinutes(10)));

        var page = await _activityAppService.GetListAsync(new GetActivitiesInput
        {
            SkipCount = 0,
            MaxResultCount = 2
        });

        page.TotalCount.ShouldBe(3);
        page.Items.Count.ShouldBe(2);
        page.Items[0].StartedAt.ShouldBe(baseTime.AddHours(2));
        page.Items[1].StartedAt.ShouldBe(baseTime.AddHours(1));
    }

    [Fact]
    public async Task GetList_Should_Filter_By_Project_And_Date_Range()
    {
        var baseTime = BaseDay.AddHours(8);
        var projectA = Guid.NewGuid();
        var projectB = Guid.NewGuid();

        var inputA1 = NewInput(startedAt: baseTime, endedAt: baseTime.AddMinutes(10));
        inputA1.ProjectId = projectA;
        var inputA2 = NewInput(startedAt: baseTime.AddDays(2), endedAt: baseTime.AddDays(2).AddMinutes(10));
        inputA2.ProjectId = projectA;
        // Same member, different project: must not overlap inputA1 (overlapping blocks are rejected).
        var inputB = NewInput(startedAt: baseTime.AddHours(1), endedAt: baseTime.AddHours(1).AddMinutes(10));
        inputB.ProjectId = projectB;

        await SubmitAsync(inputA1);
        await SubmitAsync(inputA2);
        await SubmitAsync(inputB);

        var byProject = await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = projectA });
        byProject.TotalCount.ShouldBe(2);
        byProject.Items.ShouldAllBe(a => a.ProjectId == projectA);

        var byRange = await _activityAppService.GetListAsync(new GetActivitiesInput
        {
            ProjectId = projectA,
            From = baseTime.AddDays(1),
            To = baseTime.AddDays(3)
        });
        byRange.TotalCount.ShouldBe(1);
        byRange.Items[0].StartedAt.ShouldBe(baseTime.AddDays(2));
    }

    [Fact]
    public async Task Create_Should_Persist_Screen_And_Webcam_Captures_To_Blob_Storage()
    {
        // 1x1 transparent PNG / tiny JPEG-ish payload — content just has to round-trip.
        var screenBytes = Convert.FromBase64String(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==");
        var webcamBytes = new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02, 0x03, 0xFF, 0xD9 };

        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(screenBytes);
        input.WebcamJpgBase64 = Convert.ToBase64String(webcamBytes);

        // Webcam capture is opt-in per project.
        var created = await SubmitAsync(input, p => p.AllowWebcam = true);

        var captures = await _activityAppService.GetScreenshotsAsync(created.Id);
        captures.Count.ShouldBe(2);

        var screen = captures.Single(c => c.Kind == Screenshot.ScreenKind);
        screen.ContentType.ShouldBe("image/png");
        screen.SizeBytes.ShouldBe(screenBytes.Length);
        screen.CapturedAt.ShouldBe(input.EndedAt);

        var webcam = captures.Single(c => c.Kind == Screenshot.WebcamKind);
        webcam.ContentType.ShouldBe("image/jpeg");
        webcam.SizeBytes.ShouldBe(webcamBytes.Length);

        var screenContent = await _activityAppService.GetScreenshotContentAsync(screen.Id);
        screenContent.Bytes.ShouldBe(screenBytes);
        screenContent.ContentType.ShouldBe("image/png");

        var webcamContent = await _activityAppService.GetScreenshotContentAsync(webcam.Id);
        webcamContent.Bytes.ShouldBe(webcamBytes);
    }

    [Fact]
    public async Task Create_Should_Not_Store_Captures_When_None_Are_Sent()
    {
        var created = await SubmitAsync(NewInput());

        var captures = await _activityAppService.GetScreenshotsAsync(created.Id);
        captures.ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Record_Activity_And_Skip_An_Invalid_Capture()
    {
        // A malformed/oversized capture must never cost the time block or block the agent's
        // offline queue — the activity is still recorded, the bad capture is simply dropped.
        var input = NewInput();
        input.ScreenshotPngBase64 = "not-valid-base64!!!";

        var created = await SubmitAsync(input);

        created.Id.ShouldNotBe(Guid.Empty);
        var captures = await _activityAppService.GetScreenshotsAsync(created.Id);
        captures.ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Reject_Activity_For_A_Project_The_User_Is_Not_A_Member_Of()
    {
        // No SubmitAsync -> no membership seeded, so the service must refuse. A business error (HTTP 403
        // Tracker:NotProjectMember), so agents can tell it from a missing permission and refresh their projects.
        var input = NewInput();

        var exception = await Should.ThrowAsync<BusinessException>(
            () => _activityAppService.CreateAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.NotProjectMember);
        exception.Data["projectId"].ShouldBe(input.ProjectId);
    }

    // ---- Ingest hardening: project policy, time validation, idempotency scoping, input limits ----

    private static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000001");

    [Fact]
    public async Task Create_Should_Reject_An_Archived_Project()
    {
        var input = NewInput();

        // Archived before the block even started.
        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input, p => p.Archive(input.StartedAt.AddHours(-1))));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ProjectArchived);
    }

    [Fact]
    public async Task Create_Should_Keep_The_Time_But_Drop_Everything_The_Project_Does_Not_Allow()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Png());
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg());
        input.InjectedInputEvents = 4;

        var created = await SubmitAsync(input, p =>
        {
            p.AllowScreenshot = false;
            p.AllowWebcam = false;
            p.AllowMouse = false;
            p.AllowKeyboard = false;
            p.AllowActiveWindow = false;
            p.AllowRunningPrograms = false;
        });

        created.Id.ShouldNotBe(Guid.Empty);
        created.StartedAt.ShouldBe(input.StartedAt);
        created.EndedAt.ShouldBe(input.EndedAt);
        // Neither keyboard nor mouse may be captured: activity % is "not measured", not 0% and not the agent's figure.
        created.Productivity.ShouldBeNull();
        created.InjectedInputEvents.ShouldBeNull();
        created.MouseClicks.ShouldBe(0);
        created.KeyboardHits.ShouldBe(0);
        created.ActiveWindowsJson.ShouldBe("[]");
        created.RunningProgramsJson.ShouldBe("[]");
        (await _activityAppService.GetScreenshotsAsync(created.Id)).ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Keep_Productivity_When_One_Kind_Of_Input_Is_Allowed()
    {
        var input = NewInput();

        var created = await SubmitAsync(input, p => p.AllowKeyboard = false);

        created.Productivity.ShouldBe(80);
        created.KeyboardHits.ShouldBe(0);
        created.MouseClicks.ShouldBe(120);
    }

    [Fact]
    public async Task Create_Should_Store_Only_The_Screenshot_When_The_Webcam_Is_Not_Allowed()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Png());
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg());

        var created = await SubmitAsync(input); // defaults: screenshots on, webcam off

        var captures = await _activityAppService.GetScreenshotsAsync(created.Id);
        captures.ShouldHaveSingleItem().Kind.ShouldBe(Screenshot.ScreenKind);
    }

    [Fact]
    public async Task Create_Should_Truncate_Overlong_Window_Titles()
    {
        var input = NewInput();
        input.ActiveWindows = new List<WindowInfoDto> { new() { AppName = "code.exe", WindowTitle = new string('x', 5000) } };

        var created = await SubmitAsync(input);

        created.ActiveWindowsJson.Length.ShouldBeLessThan(1000);
        created.ActiveWindowsJson.ShouldContain("code.exe");
    }

    [Fact]
    public async Task Create_Should_Reject_A_Block_Ending_In_The_Future()
    {
        var start = DateTime.UtcNow;
        var input = NewInput(startedAt: start, endedAt: start.AddMinutes(10)); // > 5 min ahead of the server

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityInFuture);
        exception.Data["serverNow"].ShouldNotBeNull();
    }

    [Fact]
    public async Task Create_Should_Tolerate_Small_Device_Clock_Skew()
    {
        var end = DateTime.UtcNow.AddMinutes(2);
        var input = NewInput(startedAt: end.AddMinutes(-10), endedAt: end);

        var created = await SubmitAsync(input);

        created.EndedAt.ShouldBe(end);
    }

    [Fact]
    public async Task Create_Should_Cap_Block_Length_At_120_Minutes_Whatever_The_Interval()
    {
        // The cap is fixed: a 10-minute project still accepts 120-minute blocks (queued under an older, longer interval).
        var start = BaseDay.AddDays(1).AddHours(8);
        var tooLong = NewInput(startedAt: start, endedAt: start.AddMinutes(121));

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(tooLong, p => p.IntervalMinutes = 10));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTooLong);
        exception.Data["maxMinutes"].ShouldBe(120);

        var atLimit = NewInput(startedAt: start, endedAt: start.AddMinutes(120));
        atLimit.ProjectId = tooLong.ProjectId;
        (await SubmitAsync(atLimit)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Accept_Blocks_Cut_Under_An_Earlier_Longer_Interval()
    {
        // An admin lowered the interval from 30 to 5 minutes; blocks the agents already cut keep being accepted.
        var start = BaseDay.AddDays(1).AddHours(8);
        var input = NewInput(startedAt: start, endedAt: start.AddMinutes(30));

        (await SubmitAsync(input, p => p.IntervalMinutes = 5)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Reject_A_Block_Overlapping_Tracked_Time_By_More_Than_A_Minute()
    {
        var start = BaseDay.AddDays(1).AddHours(9);
        var first = NewInput(startedAt: start, endedAt: start.AddMinutes(10));
        await SubmitAsync(first);

        // Same member, different project, 5 minutes of overlap -> double-counted time.
        var overlapping = NewInput(startedAt: start.AddMinutes(5), endedAt: start.AddMinutes(15));

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(overlapping));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityOverlaps);
    }

    [Fact]
    public async Task Create_Should_Detect_An_Overlap_With_A_Long_Block_That_Started_Much_Earlier()
    {
        // The overlap scan is bounded to blocks starting at most 120 minutes earlier, which still covers a stored
        // block of the maximum length that reaches into the new one.
        var start = BaseDay.AddDays(1).AddHours(9);
        var longBlock = NewInput(startedAt: start, endedAt: start.AddMinutes(120));
        await SubmitAsync(longBlock);

        var inside = NewInput(startedAt: start.AddMinutes(110), endedAt: start.AddMinutes(118));
        inside.ProjectId = longBlock.ProjectId;

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(inside));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityOverlaps);

        var after = NewInput(startedAt: start.AddMinutes(120), endedAt: start.AddMinutes(130));
        after.ProjectId = longBlock.ProjectId;
        (await SubmitAsync(after)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Allow_Up_To_A_Minute_Of_Overlap_And_Other_Members_Time()
    {
        var start = BaseDay.AddDays(1).AddHours(9);
        var first = NewInput(startedAt: start, endedAt: start.AddMinutes(10));
        await SubmitAsync(first);

        // Another member tracked the very same window — irrelevant to this member.
        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(new Activity(
            _guidGenerator.Create(), TrackerTestConsts.WorkspaceId, OtherUserId, first.ProjectId, Guid.NewGuid(),
            start.AddMinutes(10), start.AddMinutes(20))));

        // Starts 60 s before the previous block ends: tolerated jitter.
        var adjacent = NewInput(startedAt: start.AddMinutes(9), endedAt: start.AddMinutes(19));
        adjacent.ProjectId = first.ProjectId;

        (await SubmitAsync(adjacent)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Idempotency_Lookup_Should_Never_Return_Another_Members_Activity()
    {
        var clientId = Guid.NewGuid();
        var input = NewInput(clientActivityId: clientId);
        await EnsureProjectAsync(input.ProjectId);

        var foreignId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(new Activity(
            foreignId, TrackerTestConsts.WorkspaceId, OtherUserId, input.ProjectId, clientId,
            input.StartedAt.AddHours(-3), input.StartedAt.AddHours(-3).AddMinutes(10))));

        // The other member's block is neither handed back nor duplicated: the reused id is refused (409),
        // here for host users (no tenant) as well.
        var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityClientIdConflict);
        exception.Data.Values.Cast<object?>().ShouldNotContain(foreignId);

        await WithUnitOfWorkAsync(async () =>
        {
            (await _activityRepository.CountAsync(a => a.ClientActivityId == clientId)).ShouldBe(1);
        });
    }

    [Fact]
    public async Task Create_Should_Refuse_A_Client_Id_Already_Used_By_Another_Member_Of_The_Tenant()
    {
        var tenantId = Guid.NewGuid();
        var clientId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var start = BaseDay.AddDays(1).AddHours(9);

        using (GetRequiredService<ICurrentWorkspace>().Change(tenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(new Project(projectId, tenantId, "Tenant Project"));
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), tenantId, projectId, CurrentUserId));
                await _activityRepository.InsertAsync(new Activity(
                    _guidGenerator.Create(), tenantId, OtherUserId, projectId, clientId, start, start.AddMinutes(10)));
            });

            var input = NewInput(clientActivityId: clientId, startedAt: start.AddHours(1), endedAt: start.AddHours(1).AddMinutes(10));
            input.ProjectId = projectId;

            // Client ids are unique per member, and one owned by another member is refused (409) rather than
            // stored twice; the service must not hand back the other block either.
            var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
            exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityClientIdConflict);
        }
    }

    [Fact]
    public async Task Create_Should_Accept_The_Same_Client_Id_From_Members_Of_Different_Workspaces()
    {
        // Uniqueness is per (tenant, member, client id): another workspace's block never conflicts.
        var clientId = Guid.NewGuid();
        var input = NewInput(clientActivityId: clientId);
        await EnsureProjectAsync(input.ProjectId);

        var otherTenantId = Guid.NewGuid();
        using (GetRequiredService<ICurrentWorkspace>().Change(otherTenantId))
        {
            await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(new Activity(
                _guidGenerator.Create(), otherTenantId, OtherUserId, Guid.NewGuid(), clientId,
                input.StartedAt, input.EndedAt)));
        }

        var created = await _activityAppService.CreateAsync(input);

        created.UserId.ShouldBe(CurrentUserId);
        created.ClientActivityId.ShouldBe(clientId);
    }

    [Fact]
    public async Task Create_Should_Reject_Negative_Counts()
    {
        var input = NewInput();
        input.MouseClicks = -1;

        await Should.ThrowAsync<Volo.Abp.Validation.AbpValidationException>(() => SubmitAsync(input));
    }

    [Fact]
    public async Task Create_Should_Reject_Oversized_Window_Lists()
    {
        var input = NewInput();
        input.RunningPrograms = Enumerable.Range(0, CreateActivityDto.MaxWindowEntries + 1)
            .Select(i => new WindowInfoDto { AppName = $"app{i}.exe" })
            .ToList();

        await Should.ThrowAsync<Volo.Abp.Validation.AbpValidationException>(() => SubmitAsync(input));
    }

    // ---- The rest of the contract's ingest rules: who may upload, archived projects, upload window, subscription ----

    [Fact]
    public async Task Create_Should_Refuse_A_Member_Whose_Project_Role_Is_Client()
    {
        // Client stakeholders follow a project's progress; they never track time on it.
        var input = NewInput();
        await EnsureProjectAsync(input.ProjectId, role: "client");

        var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.NotProjectMember);
    }

    [Fact]
    public async Task Create_Should_Refuse_A_Deleted_Project_As_Not_A_Member()
    {
        // The membership outlived its project: the agent must refresh its projects (403), not retry forever (404).
        var input = NewInput();
        await EnsureProjectAsync(input.ProjectId);
        await WithUnitOfWorkAsync(() => _projectRepository.DeleteAsync(input.ProjectId));

        var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.NotProjectMember);
    }

    [Fact]
    public async Task Create_Should_Accept_A_Block_That_Started_Before_The_Project_Was_Archived()
    {
        // Queued offline before an admin archived the project: the recorded time is still accepted.
        var input = NewInput();

        var created = await SubmitAsync(input, p => p.Archive(input.StartedAt.AddMinutes(5)));

        created.Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Refuse_An_Archived_Project_Without_An_Archive_Time()
    {
        var input = NewInput();

        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input, p => p.IsArchived = true));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ProjectArchived);
    }

    [Fact]
    public async Task Create_Should_Reject_A_Block_That_Started_More_Than_45_Days_Ago()
    {
        var projectId = Guid.NewGuid();
        var tooOld = NewInput(startedAt: DateTime.UtcNow.AddDays(-46));
        tooOld.ProjectId = projectId;

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(tooOld));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTooOld);

        var inWindow = NewInput(startedAt: DateTime.UtcNow.AddDays(-44));
        inWindow.ProjectId = projectId;
        (await SubmitAsync(inWindow)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Refuse_Blocks_While_The_Workspace_Subscription_Is_Inactive()
    {
        var tenantId = Guid.NewGuid();
        var projectId = Guid.NewGuid();

        using (GetRequiredService<ICurrentWorkspace>().Change(tenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(new Project(projectId, tenantId, "Tenant Project"));
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), tenantId, projectId, CurrentUserId));
                await GetRequiredService<IRepository<Subscription, Guid>>().InsertAsync(
                    new Subscription(_guidGenerator.Create(), tenantId, Guid.NewGuid()) { Status = Subscription.PastDueStatus });
            });

            var input = NewInput();
            input.ProjectId = projectId;

            var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
            exception.Code.ShouldBe(TrackerDomainErrorCodes.SubscriptionInactive);
        }
    }

    [Fact]
    public async Task Create_Should_Return_The_Stored_Block_On_A_Retry_After_The_Project_Was_Archived()
    {
        // The first response was lost; by the time the agent retries, the project is archived. The block is stored.
        var input = NewInput();
        var first = await SubmitAsync(input);

        await WithUnitOfWorkAsync(async () =>
        {
            var project = await _projectRepository.GetAsync(input.ProjectId);
            project.Archive(input.StartedAt.AddMinutes(-30));
        });

        var replayed = await _activityAppService.CreateAsync(input);

        replayed.Id.ShouldBe(first.Id);
    }

    // ---- Normalization of the optional block context ----

    [Fact]
    public async Task Create_Should_Normalize_The_Device_And_Agent_Context()
    {
        var deviceId = Guid.NewGuid();
        var input = NewInput();
        input.Platform = " Windows ";
        input.OsVersion = "  Windows 11 Pro 23H2 (10.0.22631) " + new string('x', 100);
        input.AgentVersion = "0.2.0";
        input.DeviceId = deviceId;
        input.DeviceName = "DESKTOP\u0007-42\n";
        input.TimeZone = " Asia/Dhaka ";
        input.UtcOffsetMinutes = 360;
        input.EndReason = "IDLE";
        input.InjectedInputEvents = 12;

        var created = await SubmitAsync(input);

        created.Platform.ShouldBe("windows");
        created.OsVersion!.Length.ShouldBe(ActivityConsts.MaxOsVersionLength);
        created.OsVersion.ShouldStartWith("Windows 11 Pro 23H2 (10.0.22631) x");
        created.AgentVersion.ShouldBe("0.2.0");
        created.DeviceId.ShouldBe(deviceId);
        created.DeviceName.ShouldBe("DESKTOP-42");
        created.TimeZone.ShouldBe("Asia/Dhaka");
        created.UtcOffsetMinutes.ShouldBe(360);
        created.EndReason.ShouldBe("idle");
        created.InjectedInputEvents.ShouldBe(12);

        var stored = await GetStoredAsync(created.Id);
        stored.Platform.ShouldBe("windows");
        stored.TimeZone.ShouldBe("Asia/Dhaka");
    }

    [Fact]
    public async Task Create_Should_Store_Unusable_Context_Values_As_Null_And_Keep_The_Block()
    {
        var input = NewInput();
        input.Platform = "freebsd";
        input.OsVersion = "   ";
        input.AgentVersion = "\u0001\u0002";
        input.DeviceId = Guid.Empty;
        input.TimeZone = "Mars/Olympus_Mons";
        input.UtcOffsetMinutes = 900;
        input.EndReason = "crashed";
        input.IdleSeconds = -5;
        input.InjectedInputEvents = -1;

        var created = await SubmitAsync(input);

        created.Id.ShouldNotBe(Guid.Empty);
        created.Platform.ShouldBeNull();
        created.OsVersion.ShouldBeNull();
        created.AgentVersion.ShouldBeNull();
        created.DeviceId.ShouldBeNull();
        created.TimeZone.ShouldBeNull();
        created.UtcOffsetMinutes.ShouldBeNull();
        created.EndReason.ShouldBeNull();
        created.IdleSeconds.ShouldBeNull();
        created.InjectedInputEvents.ShouldBeNull();
    }

    [Fact]
    public async Task Create_Should_Accept_And_Store_Linux_Platform()
    {
        var input = NewInput();
        input.Platform = "linux";

        var created = await SubmitAsync(input);

        created.Id.ShouldNotBe(Guid.Empty);
        created.Platform.ShouldBe("linux");
    }

    [Fact]
    public async Task Create_Should_Accept_The_Extreme_Utc_Offsets()
    {
        var east = NewInput();
        east.UtcOffsetMinutes = 840;
        var west = NewInput(startedAt: BaseDay.AddHours(12));
        west.ProjectId = east.ProjectId;
        west.UtcOffsetMinutes = -840;

        (await SubmitAsync(east)).UtcOffsetMinutes.ShouldBe(840);
        (await SubmitAsync(west)).UtcOffsetMinutes.ShouldBe(-840);
    }

    [Fact]
    public async Task Create_Should_Clamp_Idle_Seconds_To_The_Block_Length()
    {
        var input = NewInput(); // 10 minutes
        input.IdleSeconds = 4000;

        (await SubmitAsync(input)).IdleSeconds.ShouldBe(600);
    }

    [Fact]
    public async Task Create_Should_Keep_The_Full_Length_Of_A_Block_Kept_After_An_Idle_Prompt()
    {
        var input = NewInput();
        input.Productivity = 0;
        input.IdleSeconds = 600;
        input.EndReason = "idle";
        input.ActiveWindows = new List<WindowInfoDto>();
        input.RunningPrograms = new List<WindowInfoDto>();

        var created = await SubmitAsync(input);

        created.IdleSeconds.ShouldBe(600);
        created.Productivity.ShouldBe(0);
        created.EndReason.ShouldBe("idle");
    }

    [Fact]
    public async Task Create_Should_Derive_Idle_Seconds_From_The_Timeline()
    {
        // 10 min 29 s: buckets 0..9, the last one 89 s long. Buckets 3 and 9 had no input: 60 + 89 s idle.
        var start = BaseDay.AddHours(10);
        var input = NewInput(startedAt: start, endedAt: start.AddSeconds(629));
        input.IdleSeconds = null;
        input.Timeline = Enumerable.Range(0, 10)
            .Select(i => new ActivityMinuteDto { Minute = i, Active = i != 3 && i != 9, KeyboardHits = i == 3 || i == 9 ? 0 : 5 })
            .ToList();

        (await SubmitAsync(input)).IdleSeconds.ShouldBe(149);
    }

    [Fact]
    public async Task Create_Should_Not_Measure_Idle_Time_Without_A_Timeline()
    {
        var input = NewInput();
        input.IdleSeconds = null;

        (await SubmitAsync(input)).IdleSeconds.ShouldBeNull();
    }

    [Fact]
    public async Task Create_Should_Not_Derive_Idle_Seconds_When_The_Project_Measures_No_Input()
    {
        // Neither keyboard nor mouse is captured, so the timeline's activity flags are no measurement of idling.
        var input = NewInput();
        input.Timeline = new List<ActivityMinuteDto> { new() { Minute = 0, Active = false } };

        var created = await SubmitAsync(input, p =>
        {
            p.AllowKeyboard = false;
            p.AllowMouse = false;
        });

        created.IdleSeconds.ShouldBeNull();
    }

    [Fact]
    public async Task Create_Should_Store_The_Clock_Skew_Of_The_Device()
    {
        // The device clock is 90 s behind the server's (positive skew).
        var input = NewInput();
        input.SentAt = DateTime.UtcNow.AddSeconds(-90);

        var created = await SubmitAsync(input);

        created.ClockSkewSeconds.ShouldNotBeNull();
        created.ClockSkewSeconds!.Value.ShouldBeInRange(88, 100);
    }

    [Fact]
    public async Task Create_Should_Clamp_The_Clock_Skew_To_Ten_Days()
    {
        var behind = NewInput();
        behind.SentAt = DateTime.UtcNow.AddDays(-30);
        var ahead = NewInput(startedAt: BaseDay.AddHours(12));
        ahead.ProjectId = behind.ProjectId;
        ahead.SentAt = DateTime.UtcNow.AddDays(30);

        (await SubmitAsync(behind)).ClockSkewSeconds.ShouldBe(10 * 24 * 3600);
        (await SubmitAsync(ahead)).ClockSkewSeconds.ShouldBe(-10 * 24 * 3600);
    }

    [Fact]
    public async Task Create_Should_Clean_Up_The_Task_Note()
    {
        var input = NewInput();
        input.Description = "  Fixing\tthe   login\r\nbug \u0007 ";

        (await SubmitAsync(input)).Description.ShouldBe("Fixing the login bug");
    }

    [Fact]
    public async Task Create_Should_Keep_Active_Seconds_In_The_Stored_Timeline()
    {
        var input = NewInput();
        input.Timeline = new List<ActivityMinuteDto>
        {
            new() { Minute = 0, KeyboardHits = 5, MouseClicks = 2, Active = true, ActiveSeconds = 42 },
            new() { Minute = 1, Active = false, ActiveSeconds = 0 },
            new() { Minute = 2, Active = true, ActiveSeconds = 95 },  // more than a bucket can hold: not a measurement
            new() { Minute = 3, Active = true, ActiveSeconds = -1 },
            new() { Minute = 4, Active = true }                     // older agent: no active seconds
        };

        var created = await SubmitAsync(input);

        created.TimelineJson.ShouldBe(
            "[{\"Minute\":0,\"KeyboardHits\":5,\"MouseClicks\":2,\"Active\":true,\"ActiveSeconds\":42}," +
            "{\"Minute\":1,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":false,\"ActiveSeconds\":0}," +
            "{\"Minute\":2,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":true}," +
            "{\"Minute\":3,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":true}," +
            "{\"Minute\":4,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":true}]");
    }

    [Fact]
    public async Task Create_Should_Store_Running_Programs_By_Name_Only()
    {
        // Titles of background windows are never stored, whatever an (older) agent sends; each app is listed once.
        var input = NewInput();
        input.RunningPrograms = new List<WindowInfoDto>
        {
            new() { AppName = "chrome.exe", WindowTitle = "Inbox - jane@acme.test - Gmail", Seconds = 30 },
            new() { AppName = "EXCEL.EXE", WindowTitle = "Salary review.xlsx" },
            new() { AppName = "Chrome.exe", WindowTitle = "Bank" },
            new() { AppName = "  ", WindowTitle = "nameless" },
            new() { AppName = "slack.exe", WindowTitle = "" }
        };

        var created = await SubmitAsync(input);

        created.RunningProgramsJson.ShouldBe(
            "[{\"AppName\":\"chrome.exe\"},{\"AppName\":\"EXCEL.EXE\"},{\"AppName\":\"slack.exe\"}]");
    }

    // ---- Rich block detail: per-window usage, per-minute timeline, thumbnails, capture refs ----

    private static readonly byte[] ScreenBytes = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01 };
    private static readonly byte[] ThumbBytes = { 0xFF, 0xD8, 0xFF, 0xE0, 0x0A, 0x0B, 0xFF, 0xD9 };
    private static readonly byte[] WebcamBytes = { 0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02, 0xFF, 0xD9 };

    [Fact]
    public async Task Create_Should_Accept_A_Legacy_Payload_Without_The_New_Fields()
    {
        var created = await SubmitAsync(NewInput());

        // Old agents: one focused window, no usage fields -> the original stored shape, untouched.
        created.ActiveWindowsJson.ShouldBe("[{\"AppName\":\"code.exe\",\"WindowTitle\":\"ActivityAppService.cs\"}]");
        created.TimelineJson.ShouldBe("[]");
        created.Captures.ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Store_Sanitized_Per_Window_Usage()
    {
        var input = NewInput(); // 10-minute block = 600 s
        input.ActiveWindows = new List<WindowInfoDto>
        {
            new() { AppName = "code.exe", WindowTitle = "A.cs", Seconds = 400, KeyboardHits = 300, MouseClicks = 20 },
            new() { AppName = "chrome.exe", WindowTitle = "Docs", Seconds = -5, KeyboardHits = -1, MouseClicks = -2 },
            new() { AppName = "slack.exe", WindowTitle = "General", Seconds = 5000 },
            new() { AppName = new string('a', 300), WindowTitle = "legacy" }
        };

        var created = await SubmitAsync(input);

        created.ActiveWindowsJson.ShouldBe(
            "[{\"AppName\":\"code.exe\",\"WindowTitle\":\"A.cs\",\"Seconds\":400,\"KeyboardHits\":300,\"MouseClicks\":20}," +
            "{\"AppName\":\"chrome.exe\",\"WindowTitle\":\"Docs\",\"Seconds\":0,\"KeyboardHits\":0,\"MouseClicks\":0}," + // negatives clamped
            "{\"AppName\":\"slack.exe\",\"WindowTitle\":\"General\",\"Seconds\":600}," + // capped at the block length
            "{\"AppName\":\"" + new string('a', 256) + "\",\"WindowTitle\":\"legacy\"}]"); // absent usage stays absent
    }

    [Fact]
    public async Task Create_Should_Drop_Per_Window_Input_Counts_The_Project_Does_Not_Allow()
    {
        var input = NewInput();
        input.ActiveWindows = new List<WindowInfoDto>
        {
            new() { AppName = "code.exe", WindowTitle = "A.cs", Seconds = 400, KeyboardHits = 300, MouseClicks = 20 }
        };

        var created = await SubmitAsync(input, p =>
        {
            p.AllowKeyboard = false;
            p.AllowMouse = false;
        });

        // Seconds are active-window data (still allowed); the input counts are not stored at all.
        created.ActiveWindowsJson.ShouldBe("[{\"AppName\":\"code.exe\",\"WindowTitle\":\"A.cs\",\"Seconds\":400}]");
    }

    [Fact]
    public async Task Create_Should_Store_A_Sanitized_Timeline()
    {
        var input = NewInput(); // 10-minute block: buckets 0..9, plus one bucket of slack (10)
        input.Timeline = new List<ActivityMinuteDto>
        {
            new() { Minute = 10, KeyboardHits = 1, MouseClicks = 1, Active = true },
            new() { Minute = -1, KeyboardHits = 7, MouseClicks = 7, Active = true },
            new() { Minute = 0, KeyboardHits = 5, MouseClicks = 2, Active = true, AppName = "code.exe" },
            new() { Minute = 3, KeyboardHits = -4, MouseClicks = -1, Active = false, AppName = new string('b', 300) },
            new() { Minute = 3, KeyboardHits = 99, MouseClicks = 99, Active = true }, // duplicate bucket: first wins
            new() { Minute = 11, KeyboardHits = 8, MouseClicks = 8, Active = true }   // past the block
        };

        var created = await SubmitAsync(input);

        created.TimelineJson.ShouldBe(
            "[{\"Minute\":0,\"KeyboardHits\":5,\"MouseClicks\":2,\"Active\":true,\"AppName\":\"code.exe\"}," +
            "{\"Minute\":3,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":false,\"AppName\":\"" + new string('b', 256) + "\"}," +
            "{\"Minute\":10,\"KeyboardHits\":1,\"MouseClicks\":1,\"Active\":true}]");

        var listed = await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = input.ProjectId });
        listed.Items.ShouldHaveSingleItem().TimelineJson.ShouldBe(created.TimelineJson);
    }

    [Fact]
    public async Task Create_Should_Blank_Timeline_Data_The_Project_Does_Not_Allow()
    {
        var input = NewInput();
        input.Timeline = new List<ActivityMinuteDto>
        {
            new() { Minute = 0, KeyboardHits = 5, MouseClicks = 2, Active = true, AppName = "code.exe" }
        };

        var created = await SubmitAsync(input, p =>
        {
            p.AllowKeyboard = false;
            p.AllowMouse = false;
            p.AllowActiveWindow = false;
        });

        // The minute and its active flag are time data and always kept.
        created.TimelineJson.ShouldBe("[{\"Minute\":0,\"KeyboardHits\":0,\"MouseClicks\":0,\"Active\":true}]");
    }

    [Fact]
    public async Task Create_Should_Reject_An_Oversized_Timeline()
    {
        var input = NewInput();
        input.Timeline = Enumerable.Range(0, CreateActivityDto.MaxTimelineEntries + 1)
            .Select(i => new ActivityMinuteDto { Minute = i })
            .ToList();

        await Should.ThrowAsync<Volo.Abp.Validation.AbpValidationException>(() => SubmitAsync(input));
    }

    [Fact]
    public async Task Create_Should_Store_The_Thumbnail_But_Keep_It_Out_Of_The_Screenshot_List()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(ScreenBytes);
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(ThumbBytes);

        var created = await SubmitAsync(input);

        created.Captures.Select(c => c.Kind).ShouldBe(new[] { Screenshot.ScreenKind, Screenshot.ThumbKind });

        // Thumbnails are a derived rendition: the full-size capture list only has the screen.
        var screenshots = await _activityAppService.GetScreenshotsAsync(created.Id);
        screenshots.ShouldHaveSingleItem().Kind.ShouldBe(Screenshot.ScreenKind);
        screenshots[0].Id.ShouldBe(created.Captures[0].Id);

        // ... but it is still served by id.
        var thumb = await _activityAppService.GetScreenshotContentAsync(created.Captures[1].Id);
        thumb.Bytes.ShouldBe(ThumbBytes);
        thumb.ContentType.ShouldBe("image/jpeg");

        // An idempotent retry hands back the same capture references.
        var replayed = await SubmitAsync(input);
        replayed.Id.ShouldBe(created.Id);
        replayed.Captures.Select(c => c.Id).ShouldBe(created.Captures.Select(c => c.Id));
    }

    [Fact]
    public async Task Create_Should_Drop_The_Thumbnail_When_Screenshots_Are_Not_Allowed()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(ScreenBytes);
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(ThumbBytes);

        var created = await SubmitAsync(input, p => p.AllowScreenshot = false);

        created.Captures.ShouldBeEmpty();
        await WithUnitOfWorkAsync(async () =>
        {
            var stored = await GetRequiredService<IRepository<Screenshot, Guid>>()
                .CountAsync(s => s.ActivityId == created.Id);
            stored.ShouldBe(0);
        });
    }

    [Fact]
    public async Task GetList_Should_Return_The_Capture_References_Of_Each_Activity()
    {
        var baseTime = BaseDay.AddDays(2).AddHours(9);
        var withCaptures = NewInput(startedAt: baseTime, endedAt: baseTime.AddMinutes(10));
        withCaptures.ScreenshotPngBase64 = Convert.ToBase64String(ScreenBytes);
        withCaptures.ScreenshotThumbJpgBase64 = Convert.ToBase64String(ThumbBytes);
        withCaptures.WebcamJpgBase64 = Convert.ToBase64String(WebcamBytes);
        var withoutCaptures = NewInput(startedAt: baseTime.AddMinutes(10), endedAt: baseTime.AddMinutes(20));
        withoutCaptures.ProjectId = withCaptures.ProjectId;

        var created = await SubmitAsync(withCaptures, p => p.AllowWebcam = true);
        await SubmitAsync(withoutCaptures);

        var page = await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = withCaptures.ProjectId });

        page.Items.Count.ShouldBe(2);
        page.Items[0].ClientActivityId.ShouldBe(withoutCaptures.ClientActivityId); // newest first
        page.Items[0].Captures.ShouldBeEmpty();

        var captures = page.Items[1].Captures;
        captures.Select(c => c.Kind).ShouldBe(new[] { Screenshot.ScreenKind, Screenshot.ThumbKind, Screenshot.WebcamKind });
        captures.Select(c => c.Id).ShouldBe(created.Captures.Select(c => c.Id));

        var fullSize = await _activityAppService.GetScreenshotsAsync(created.Id);
        fullSize.Select(s => s.Id).ShouldBe(
            captures.Where(c => c.Kind != Screenshot.ThumbKind).Select(c => c.Id), ignoreOrder: true);
    }

    [Fact]
    public async Task Capture_References_Should_Carry_The_Capture_Time_And_Blur_Flag()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(ScreenBytes);
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(ThumbBytes);

        var created = await SubmitAsync(input);

        created.Captures.Count.ShouldBe(2);
        created.Captures.ShouldAllBe(c => c.CapturedAt == input.EndedAt && !c.Blurred);

        var listed = await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = input.ProjectId });
        var refs = listed.Items.ShouldHaveSingleItem().Captures;
        refs.Select(c => (c.Id, c.Kind, c.CapturedAt, c.Blurred))
            .ShouldBe(created.Captures.Select(c => (c.Id, c.Kind, c.CapturedAt, c.Blurred)));
    }

    [Fact]
    public async Task Create_Should_Keep_Productivity_Null_When_It_Was_Not_Measured()
    {
        // Input capture unavailable/denied: the agent sends no productivity, which is "not measured", never 0%.
        var input = NewInput();
        input.Productivity = null;

        var created = await SubmitAsync(input);

        created.Productivity.ShouldBeNull();
        var listed = await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = input.ProjectId });
        listed.Items.ShouldHaveSingleItem().Productivity.ShouldBeNull();
    }

    [Fact]
    public async Task Create_Should_Still_Reject_A_Productivity_Outside_0_To_100()
    {
        var input = NewInput();
        input.Productivity = 101;

        await Should.ThrowAsync<Volo.Abp.Validation.AbpValidationException>(() => SubmitAsync(input));
    }

    [Fact]
    public async Task GetList_Should_Return_The_Device_And_Agent_Context_Of_A_Block()
    {
        var projectId = Guid.NewGuid();
        await EnsureProjectAsync(projectId);
        var deviceId = Guid.NewGuid();
        var start = BaseDay.AddDays(3).AddHours(9);

        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(new Activity(
            _guidGenerator.Create(), TrackerTestConsts.WorkspaceId, CurrentUserId, projectId, Guid.NewGuid(), start, start.AddMinutes(10))
        {
            Productivity = null,
            Platform = "macos",
            OsVersion = "macOS 14.5 (23F79)",
            AgentVersion = "0.2.0",
            DeviceId = deviceId,
            DeviceName = "Ana's MacBook",
            TimeZone = "Asia/Dhaka",
            UtcOffsetMinutes = 360,
            IdleSeconds = 120,
            EndReason = "idle",
            ClockSkewSeconds = -4,
            InjectedInputEvents = 7
        }));

        var dto = (await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = projectId }))
            .Items.ShouldHaveSingleItem();

        dto.Productivity.ShouldBeNull();
        dto.Platform.ShouldBe("macos");
        dto.OsVersion.ShouldBe("macOS 14.5 (23F79)");
        dto.AgentVersion.ShouldBe("0.2.0");
        dto.DeviceId.ShouldBe(deviceId);
        dto.DeviceName.ShouldBe("Ana's MacBook");
        dto.TimeZone.ShouldBe("Asia/Dhaka");
        dto.UtcOffsetMinutes.ShouldBe(360);
        dto.IdleSeconds.ShouldBe(120);
        dto.EndReason.ShouldBe("idle");
        dto.ClockSkewSeconds.ShouldBe(-4);
        dto.InjectedInputEvents.ShouldBe(7);
    }

    [Fact]
    public async Task GetList_Should_Leave_The_Context_Of_A_Legacy_Block_Empty()
    {
        var created = await SubmitAsync(NewInput());

        var dto = (await _activityAppService.GetListAsync(new GetActivitiesInput { ProjectId = created.ProjectId }))
            .Items.ShouldHaveSingleItem();

        dto.Platform.ShouldBeNull();
        dto.OsVersion.ShouldBeNull();
        dto.AgentVersion.ShouldBeNull();
        dto.DeviceId.ShouldBeNull();
        dto.DeviceName.ShouldBeNull();
        dto.TimeZone.ShouldBeNull();
        dto.UtcOffsetMinutes.ShouldBeNull();
        dto.IdleSeconds.ShouldBeNull();
        dto.EndReason.ShouldBeNull();
        dto.ClockSkewSeconds.ShouldBeNull();
        dto.InjectedInputEvents.ShouldBeNull();
    }

    [Fact]
    public async Task GetList_Should_Page_Stably_Through_Blocks_That_Start_At_The_Same_Time()
    {
        // Blocks of several members starting at the same instant: paging must neither repeat nor skip one.
        var projectId = Guid.NewGuid();
        await EnsureProjectAsync(projectId);
        var start = BaseDay.AddDays(3).AddHours(14);
        var ids = new List<Guid>();
        await WithUnitOfWorkAsync(async () =>
        {
            for (var i = 0; i < 5; i++)
            {
                var activity = new Activity(
                    _guidGenerator.Create(), TrackerTestConsts.WorkspaceId, Guid.NewGuid(), projectId, Guid.NewGuid(), start, start.AddMinutes(10));
                ids.Add(activity.Id);
                await _activityRepository.InsertAsync(activity);
            }
        });

        var firstPass = new List<Guid>();
        var secondPass = new List<Guid>();
        for (var skip = 0; skip < ids.Count; skip += 2)
        {
            var request = new GetActivitiesInput { ProjectId = projectId, SkipCount = skip, MaxResultCount = 2 };
            var page = await _activityAppService.GetListAsync(request);
            page.TotalCount.ShouldBe(ids.Count);
            firstPass.AddRange(page.Items.Select(a => a.Id));
            secondPass.AddRange((await _activityAppService.GetListAsync(request)).Items.Select(a => a.Id));
        }

        firstPass.ShouldBe(ids, ignoreOrder: true);
        firstPass.Distinct().Count().ShouldBe(ids.Count);
        secondPass.ShouldBe(firstPass);
    }

    // ---- Captures: format detection, size limits, capture time, blur policy ----

    [Fact]
    public async Task Create_Should_Store_A_Jpeg_Screen_Capture_As_Jpeg()
    {
        // The field is named for PNG, but agents send JPEG: the bytes decide the stored type and file name.
        var bytes = Jpeg(64);
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(bytes);

        var created = await SubmitAsync(input);

        var screen = (await GetStoredCapturesAsync(created.Id)).ShouldHaveSingleItem();
        screen.ContentType.ShouldBe("image/jpeg");
        screen.StorageUrl.ShouldBe($"activities/{created.Id}/screen.jpg");

        var content = await _activityAppService.GetScreenshotContentAsync(screen.Id);
        content.Bytes.ShouldBe(bytes);
        content.ContentType.ShouldBe("image/jpeg");
        content.FileName.ShouldBe($"{created.Id}-screen.jpg");
    }

    [Fact]
    public async Task Create_Should_Store_A_Png_Screen_Capture_As_Png()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Png(64));

        var created = await SubmitAsync(input);

        var screen = (await GetStoredCapturesAsync(created.Id)).ShouldHaveSingleItem();
        screen.ContentType.ShouldBe("image/png");
        screen.StorageUrl.ShouldBe($"activities/{created.Id}/screen.png");
    }

    [Fact]
    public async Task Create_Should_Drop_A_Capture_That_Is_Neither_Jpeg_Nor_Png()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(System.Text.Encoding.UTF8.GetBytes("<svg onload=alert(1)>"));
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg());

        var created = await SubmitAsync(input);

        created.Captures.ShouldHaveSingleItem().Kind.ShouldBe(Screenshot.ThumbKind);
    }

    [Fact]
    public async Task Create_Should_Drop_Captures_Over_Their_Size_Limits_And_Keep_The_Block()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Png(ActivityAppService.MaxScreenCaptureBytes + 1));
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg(ActivityAppService.MaxThumbCaptureBytes + 1));
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg(ActivityAppService.MaxWebcamCaptureBytes + 1));

        var created = await SubmitAsync(input, p => p.AllowWebcam = true);

        created.Id.ShouldNotBe(Guid.Empty);
        created.Captures.ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Accept_Captures_At_Their_Size_Limits()
    {
        var input = NewInput();
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg(ActivityAppService.MaxThumbCaptureBytes));
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg(ActivityAppService.MaxWebcamCaptureBytes));

        var created = await SubmitAsync(input, p => p.AllowWebcam = true);

        created.Captures.Select(c => c.Kind).ShouldBe(new[] { Screenshot.ThumbKind, Screenshot.WebcamKind });
    }

    [Fact]
    public async Task Create_Should_Use_The_Reported_Screen_Capture_Time_Within_The_Block()
    {
        var input = NewInput(); // 10:00 - 10:10
        var capturedAt = input.StartedAt.AddMinutes(4).AddSeconds(17);
        input.ScreenshotCapturedAt = capturedAt;
        input.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg());
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg());

        var created = await SubmitAsync(input, p => p.AllowWebcam = true);

        created.Captures.Single(c => c.Kind == Screenshot.ScreenKind).CapturedAt.ShouldBe(capturedAt);
        created.Captures.Single(c => c.Kind == Screenshot.ThumbKind).CapturedAt.ShouldBe(capturedAt);
        created.Captures.Single(c => c.Kind == Screenshot.WebcamKind).CapturedAt.ShouldBe(input.EndedAt);
    }

    [Fact]
    public async Task Create_Should_Accept_A_Screen_Capture_Time_Up_To_A_Minute_Outside_The_Block()
    {
        var input = NewInput();
        input.ScreenshotCapturedAt = input.EndedAt.AddSeconds(60);
        input.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());

        var created = await SubmitAsync(input);

        created.Captures.ShouldHaveSingleItem().CapturedAt.ShouldBe(input.EndedAt.AddSeconds(60));
    }

    [Fact]
    public async Task Create_Should_Use_The_Block_End_For_A_Screen_Capture_Time_Outside_The_Block()
    {
        var before = NewInput();
        before.ScreenshotCapturedAt = before.StartedAt.AddSeconds(-61);
        before.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());
        var after = NewInput(startedAt: BaseDay.AddHours(12));
        after.ProjectId = before.ProjectId;
        after.ScreenshotCapturedAt = after.EndedAt.AddHours(3);
        after.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());

        (await SubmitAsync(before)).Captures.ShouldHaveSingleItem().CapturedAt.ShouldBe(before.EndedAt);
        (await SubmitAsync(after)).Captures.ShouldHaveSingleItem().CapturedAt.ShouldBe(after.EndedAt);
    }

    [Fact]
    public async Task Create_Should_Record_That_The_Agent_Blurred_The_Screenshot()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg());
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotBlurred = true;

        var created = await SubmitAsync(input, p => p.AllowWebcam = true);

        created.Captures.Single(c => c.Kind == Screenshot.ScreenKind).Blurred.ShouldBeTrue();
        created.Captures.Single(c => c.Kind == Screenshot.ThumbKind).Blurred.ShouldBeTrue();
        created.Captures.Single(c => c.Kind == Screenshot.WebcamKind).Blurred.ShouldBeFalse();
        (await _activityAppService.GetScreenshotsAsync(created.Id))
            .Single(s => s.Kind == Screenshot.ScreenKind).Blurred.ShouldBeTrue();
    }

    [Fact]
    public async Task Create_Should_Drop_Unblurred_Screenshots_When_The_Project_Requires_Blur()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg());
        input.WebcamJpgBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotBlurred = null; // an older agent never blurs

        var created = await SubmitAsync(input, p =>
        {
            p.BlurScreenshots = true;
            p.AllowWebcam = true;
        });

        // The block and the webcam frame are kept; the readable screen and its thumbnail are not.
        created.Id.ShouldNotBe(Guid.Empty);
        created.Captures.ShouldHaveSingleItem().Kind.ShouldBe(Screenshot.WebcamKind);
    }

    [Fact]
    public async Task Create_Should_Keep_Blurred_Screenshots_When_The_Project_Requires_Blur()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotThumbJpgBase64 = Convert.ToBase64String(Jpeg());
        input.ScreenshotBlurred = true;

        var created = await SubmitAsync(input, p => p.BlurScreenshots = true);

        created.Captures.Count.ShouldBe(2);
        created.Captures.ShouldAllBe(c => c.Blurred);
    }

    [Fact]
    public async Task GetScreenshotContent_Should_Serve_The_Type_Of_The_Stored_Bytes()
    {
        // Older servers stored every screen capture as image/png, also when the agent sent a JPEG.
        var created = await SubmitAsync(NewInput());
        var bytes = Jpeg(32);
        var screenshotId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            var blobName = $"activities/{created.Id}/screen.png";
            await GetRequiredService<IBlobContainer>().SaveAsync(blobName, bytes);
            await GetRequiredService<IRepository<Screenshot, Guid>>().InsertAsync(new Screenshot(
                screenshotId, TrackerTestConsts.WorkspaceId, created.Id, blobName, created.EndedAt, bytes.Length, "image/png"));
        });

        var content = await _activityAppService.GetScreenshotContentAsync(screenshotId);

        content.ContentType.ShouldBe("image/jpeg");
        content.FileName.ShouldBe($"{created.Id}-screen.jpg");
        content.Bytes.ShouldBe(bytes);
    }

    // ---- Task note (PUT /api/app/activity/{id}/note). The test caller holds every permission (Team.Manage). ----

    [Fact]
    public async Task UpdateNote_Should_Store_The_Cleaned_Up_Note()
    {
        var created = await SubmitAsync(NewInput());

        var updated = await _activityAppService.UpdateNoteAsync(created.Id, new UpdateActivityNoteDto
        {
            Description = "\n  Release   notes\tfor v0.2 \u0000"
        });

        updated.Id.ShouldBe(created.Id);
        updated.Description.ShouldBe("Release notes for v0.2");
        (await GetStoredAsync(created.Id)).Description.ShouldBe("Release notes for v0.2");
    }

    [Fact]
    public async Task UpdateNote_Should_Clear_A_Blank_Note()
    {
        var created = await SubmitAsync(NewInput());

        var updated = await _activityAppService.UpdateNoteAsync(created.Id, new UpdateActivityNoteDto { Description = " \t " });

        updated.Description.ShouldBeNull();
        (await GetStoredAsync(created.Id)).Description.ShouldBeNull();
    }

    [Fact]
    public async Task UpdateNote_Should_Let_A_Team_Manager_Change_Any_Block_At_Any_Age()
    {
        var projectId = Guid.NewGuid();
        await EnsureProjectAsync(projectId);
        var start = DateTime.UtcNow.AddDays(-60);
        var othersBlock = new Activity(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, OtherUserId, projectId, Guid.NewGuid(), start, start.AddMinutes(10));
        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(othersBlock));

        var updated = await _activityAppService.UpdateNoteAsync(othersBlock.Id, new UpdateActivityNoteDto { Description = "Client call" });

        updated.Description.ShouldBe("Client call");
        updated.UserId.ShouldBe(OtherUserId);
    }

    [Fact]
    public async Task UpdateNote_Should_Reject_A_Note_Over_2048_Characters()
    {
        var created = await SubmitAsync(NewInput());

        await Should.ThrowAsync<Volo.Abp.Validation.AbpValidationException>(() => _activityAppService.UpdateNoteAsync(
            created.Id, new UpdateActivityNoteDto { Description = new string('n', UpdateActivityNoteDto.MaxDescriptionLength + 1) }));
    }
}
