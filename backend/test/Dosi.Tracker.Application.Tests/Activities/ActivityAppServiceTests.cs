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
    private async Task<ActivityDto> SubmitAsync(CreateActivityDto input)
    {
        if (_seededProjects.Add(input.ProjectId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(
                    new Project(input.ProjectId, null, "Seeded Project"));
                await _memberRepository.InsertAsync(
                    new ProjectMember(_guidGenerator.Create(), null, input.ProjectId, CurrentUserId));
            });
        }

        return await _activityAppService.CreateAsync(input);
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
        var inputB = NewInput(startedAt: baseTime, endedAt: baseTime.AddMinutes(10));
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

        var created = await SubmitAsync(input);

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
}
