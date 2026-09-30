using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Activities;

public abstract class ActivityAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

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

    private async Task EnsureProjectAsync(Guid projectId, Action<Project>? configureProject = null)
    {
        if (_seededProjects.Add(projectId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var project = new Project(projectId, null, "Seeded Project");
                configureProject?.Invoke(project);
                await _projectRepository.InsertAsync(project);
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), null, projectId, CurrentUserId));
            });
        }
    }

    private static CreateActivityDto NewInput(
        Guid? clientActivityId = null,
        DateTime? startedAt = null,
        DateTime? endedAt = null)
    {
        var start = startedAt ?? new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);
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
        var start = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);
        var input = NewInput(startedAt: start, endedAt: start.AddMinutes(-1));

        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTimeRangeInvalid);
    }

    [Fact]
    public async Task GetList_Should_Page_And_Order_By_StartedAt_Descending()
    {
        var baseTime = new DateTime(2026, 7, 20, 8, 0, 0, DateTimeKind.Utc);
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
        var baseTime = new DateTime(2026, 7, 20, 8, 0, 0, DateTimeKind.Utc);
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
        // No SubmitAsync -> no membership seeded, so the service must refuse.
        var input = NewInput();

        await Should.ThrowAsync<AbpAuthorizationException>(
            () => _activityAppService.CreateAsync(input));
    }

    // ---- Ingest hardening: project policy, time validation, idempotency scoping, input limits ----

    private static readonly Guid OtherUserId = Guid.Parse("7a1b2c3d-0000-4000-8000-000000000001");

    [Fact]
    public async Task Create_Should_Reject_An_Archived_Project()
    {
        var input = NewInput();

        var exception = await Should.ThrowAsync<BusinessException>(
            () => SubmitAsync(input, p => p.Archive()));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ProjectArchived);
    }

    [Fact]
    public async Task Create_Should_Keep_The_Time_But_Drop_Everything_The_Project_Does_Not_Allow()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(new byte[] { 1, 2, 3 });
        input.WebcamJpgBase64 = Convert.ToBase64String(new byte[] { 4, 5, 6 });

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
        created.Productivity.ShouldBe(80);
        created.MouseClicks.ShouldBe(0);
        created.KeyboardHits.ShouldBe(0);
        created.ActiveWindowsJson.ShouldBe("[]");
        created.RunningProgramsJson.ShouldBe("[]");
        (await _activityAppService.GetScreenshotsAsync(created.Id)).ShouldBeEmpty();
    }

    [Fact]
    public async Task Create_Should_Store_Only_The_Screenshot_When_The_Webcam_Is_Not_Allowed()
    {
        var input = NewInput();
        input.ScreenshotPngBase64 = Convert.ToBase64String(new byte[] { 1, 2, 3 });
        input.WebcamJpgBase64 = Convert.ToBase64String(new byte[] { 4, 5, 6 });

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
    public async Task Create_Should_Cap_Block_Length_At_Twice_The_Project_Interval()
    {
        // Seeded project interval = 10 min -> max block = max(2 × 10, 15) = 20 min.
        var start = new DateTime(2026, 7, 21, 8, 0, 0, DateTimeKind.Utc);
        var tooLong = NewInput(startedAt: start, endedAt: start.AddMinutes(21));

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(tooLong));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTooLong);

        var atLimit = NewInput(startedAt: start, endedAt: start.AddMinutes(20));
        atLimit.ProjectId = tooLong.ProjectId;
        (await SubmitAsync(atLimit)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Use_A_15_Minute_Floor_For_Short_Intervals()
    {
        var start = new DateTime(2026, 7, 21, 8, 0, 0, DateTimeKind.Utc);
        var input = NewInput(startedAt: start, endedAt: start.AddMinutes(15)); // 2 × 5 = 10 < 15 floor

        (await SubmitAsync(input, p => p.IntervalMinutes = 5)).Id.ShouldNotBe(Guid.Empty);
    }

    [Fact]
    public async Task Create_Should_Reject_A_Block_Overlapping_Tracked_Time_By_More_Than_A_Minute()
    {
        var start = new DateTime(2026, 7, 21, 9, 0, 0, DateTimeKind.Utc);
        var first = NewInput(startedAt: start, endedAt: start.AddMinutes(10));
        await SubmitAsync(first);

        // Same member, different project, 5 minutes of overlap -> double-counted time.
        var overlapping = NewInput(startedAt: start.AddMinutes(5), endedAt: start.AddMinutes(15));

        var exception = await Should.ThrowAsync<BusinessException>(() => SubmitAsync(overlapping));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityOverlaps);
    }

    [Fact]
    public async Task Create_Should_Allow_Up_To_A_Minute_Of_Overlap_And_Other_Members_Time()
    {
        var start = new DateTime(2026, 7, 21, 9, 0, 0, DateTimeKind.Utc);
        var first = NewInput(startedAt: start, endedAt: start.AddMinutes(10));
        await SubmitAsync(first);

        // Another member tracked the very same window — irrelevant to this member.
        await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(new Activity(
            _guidGenerator.Create(), null, OtherUserId, first.ProjectId, Guid.NewGuid(),
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
            foreignId, null, OtherUserId, input.ProjectId, clientId,
            input.StartedAt.AddHours(-3), input.StartedAt.AddHours(-3).AddMinutes(10))));

        var created = await _activityAppService.CreateAsync(input);

        created.Id.ShouldNotBe(foreignId);
        created.UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task Create_Should_Refuse_A_Client_Id_Already_Used_By_Another_Member_Of_The_Tenant()
    {
        var tenantId = Guid.NewGuid();
        var clientId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var start = new DateTime(2026, 7, 21, 9, 0, 0, DateTimeKind.Utc);

        using (GetRequiredService<Volo.Abp.MultiTenancy.ICurrentTenant>().Change(tenantId))
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

            // The unique (TenantId, ClientActivityId) index fires; the service must not hand back the other block.
            var exception = await Should.ThrowAsync<BusinessException>(() => _activityAppService.CreateAsync(input));
            exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityClientIdConflict);
        }
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
        var baseTime = new DateTime(2026, 7, 22, 9, 0, 0, DateTimeKind.Utc);
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
}
