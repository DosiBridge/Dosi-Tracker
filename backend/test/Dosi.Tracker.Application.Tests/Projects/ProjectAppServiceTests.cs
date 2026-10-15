using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Teams;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.Projects;

public abstract class ProjectAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly IProjectAppService _projectAppService;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected ProjectAppServiceTests()
    {
        _projectAppService = GetRequiredService<IProjectAppService>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    /// <summary>A project (not created through the service) where the caller holds the given membership role(s).</summary>
    private async Task<Guid> SeedProjectWithCallerAsync(string title, params string[] roles)
    {
        var projectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            await _projectRepository.InsertAsync(new Project(projectId, TrackerTestConsts.WorkspaceId, title));
            foreach (var role in roles)
            {
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, projectId, CurrentUserId, role));
            }
        });
        return projectId;
    }

    private Task<ProjectDto> CreateProjectAsync(string title = "Website Redesign", int intervalMinutes = 10)
    {
        return _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = title,
            Description = "Test project",
            Color = "#112233",
            IntervalMinutes = intervalMinutes
        });
    }

    [Fact]
    public async Task Should_Create_And_Get_Project()
    {
        var created = await CreateProjectAsync();

        var fetched = await _projectAppService.GetAsync(created.Id);

        fetched.Title.ShouldBe("Website Redesign");
        fetched.Description.ShouldBe("Test project");
        fetched.Color.ShouldBe("#112233");
        fetched.IntervalMinutes.ShouldBe(10);
        fetched.IsArchived.ShouldBeFalse();
        fetched.ArchivedAt.ShouldBeNull();
        fetched.IdleThresholdMinutes.ShouldBe(5);
        fetched.BlurScreenshots.ShouldBeFalse();
    }

    [Fact]
    public async Task Project_Dtos_Should_Carry_The_Audit_Properties()
    {
        var created = await CreateProjectAsync();

        created.CreationTime.ShouldNotBe(default);
        created.CreatorId.ShouldBe(CurrentUserId);
        created.LastModificationTime.ShouldBeNull();
        (await _projectAppService.GetAsync(created.Id)).CreationTime.ShouldBe(created.CreationTime);

        var updated = await _projectAppService.UpdateAsync(created.Id, new CreateUpdateProjectDto
        {
            Title = "Website Redesign v2",
            Color = "#112233"
        });
        updated.CreationTime.ShouldBe(created.CreationTime);
        updated.LastModificationTime.ShouldNotBeNull();
        updated.LastModifierId.ShouldBe(CurrentUserId);

        // Archive/unarchive answer with the stored state, audit properties included.
        var archived = await _projectAppService.ArchiveAsync(created.Id);
        archived.LastModificationTime.ShouldNotBeNull();
        archived.LastModificationTime!.Value.ShouldBeGreaterThanOrEqualTo(updated.LastModificationTime!.Value);
        (await _projectAppService.GetAsync(created.Id)).LastModificationTime.ShouldBe(archived.LastModificationTime);

        var list = await _projectAppService.GetListAsync(new PagedAndSortedResultRequestDto { MaxResultCount = 100 });
        list.Items.Single(p => p.Id == created.Id).CreationTime.ShouldBe(created.CreationTime);
    }

    [Fact]
    public void The_Service_Itself_Should_Enforce_The_Colour_Description_And_Title_Rules()
    {
        // Callers that bypass the DTO's attributes still meet the same rules.
        static AbpValidationException Invalid(CreateUpdateProjectDto input) =>
            Should.Throw<AbpValidationException>(() => ProjectAppService.NormalizeAndValidate(input));

        Invalid(new CreateUpdateProjectDto { Title = "Colors", Color = "url(https://tracker.example/p.gif)" })
            .ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(CreateUpdateProjectDto.Color)));
        Invalid(new CreateUpdateProjectDto { Title = "Colors", Color = null! })
            .ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(CreateUpdateProjectDto.Color)));
        Invalid(new CreateUpdateProjectDto { Title = "Long", Color = "#112233", Description = new string('d', 2049) })
            .ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(CreateUpdateProjectDto.Description)));
        Invalid(new CreateUpdateProjectDto { Title = "   ", Color = "#112233" })
            .ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(CreateUpdateProjectDto.Title)));

        var valid = new CreateUpdateProjectDto { Title = "  Website  ", Color = "#AbCdEf", Description = "  Landing page \n" };
        ProjectAppService.NormalizeAndValidate(valid);
        valid.Title.ShouldBe("Website");
        valid.Description.ShouldBe("Landing page");
        valid.Color.ShouldBe("#AbCdEf");

        var blankDescription = new CreateUpdateProjectDto { Title = "Blank", Color = "#112233", Description = " \t " };
        ProjectAppService.NormalizeAndValidate(blankDescription);
        blankDescription.Description.ShouldBeNull();
    }

    [Fact]
    public async Task Create_And_Update_Should_Trim_The_Title_And_The_Description()
    {
        var created = await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "  Spaced Out  ",
            Color = "#112233",
            Description = "   "
        });

        created.Title.ShouldBe("Spaced Out");
        created.Description.ShouldBeNull();

        var updated = await _projectAppService.UpdateAsync(created.Id, new CreateUpdateProjectDto
        {
            Title = "Spaced Out",
            Color = "#112233",
            Description = "  Now described  "
        });
        updated.Description.ShouldBe("Now described");
    }

    [Fact]
    public async Task Create_And_Update_Should_Store_The_Idle_And_Blur_Policy()
    {
        var created = await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "Privacy Project",
            Color = "#112233",
            IdleThresholdMinutes = 0, // idle detection off
            BlurScreenshots = true
        });

        created.IdleThresholdMinutes.ShouldBe(0);
        created.BlurScreenshots.ShouldBeTrue();

        var updated = await _projectAppService.UpdateAsync(created.Id, new CreateUpdateProjectDto
        {
            Title = "Privacy Project",
            Color = "#112233",
            IdleThresholdMinutes = 15,
            BlurScreenshots = false
        });

        updated.IdleThresholdMinutes.ShouldBe(15);
        updated.BlurScreenshots.ShouldBeFalse();

        var fetched = await _projectAppService.GetAsync(created.Id);
        fetched.IdleThresholdMinutes.ShouldBe(15);
        fetched.BlurScreenshots.ShouldBeFalse();
    }

    [Fact]
    public async Task Create_Should_Reject_An_Idle_Threshold_Outside_0_To_60_Minutes()
    {
        await Should.ThrowAsync<AbpValidationException>(() => _projectAppService.CreateAsync(
            new CreateUpdateProjectDto { Title = "Idle", Color = "#112233", IdleThresholdMinutes = 61 }));

        await Should.ThrowAsync<AbpValidationException>(() => _projectAppService.CreateAsync(
            new CreateUpdateProjectDto { Title = "Idle", Color = "#112233", IdleThresholdMinutes = -1 }));
    }

    [Theory]
    [InlineData("url(https://tracker.example/p.gif)")]
    [InlineData("#12345")]
    [InlineData("#1234567")]
    [InlineData("blue")]
    [InlineData("#12345G")]
    public async Task Create_Should_Reject_A_Color_That_Is_Not_A_Hex_Color(string color)
    {
        await Should.ThrowAsync<AbpValidationException>(() => _projectAppService.CreateAsync(
            new CreateUpdateProjectDto { Title = "Colors", Color = color }));
    }

    [Fact]
    public async Task Create_Should_Accept_Upper_And_Lower_Case_Hex_Colors()
    {
        (await _projectAppService.CreateAsync(new CreateUpdateProjectDto { Title = "Upper", Color = "#ABCDEF" }))
            .Color.ShouldBe("#ABCDEF");
        (await _projectAppService.CreateAsync(new CreateUpdateProjectDto { Title = "Lower", Color = "#abcdef" }))
            .Color.ShouldBe("#abcdef");
    }

    [Fact]
    public async Task Create_Should_Reject_A_Description_Longer_Than_2048_Characters()
    {
        await Should.ThrowAsync<AbpValidationException>(() => _projectAppService.CreateAsync(
            new CreateUpdateProjectDto { Title = "Long", Color = "#112233", Description = new string('d', 2049) }));

        var atLimit = await _projectAppService.CreateAsync(
            new CreateUpdateProjectDto { Title = "Long", Color = "#112233", Description = new string('d', 2048) });
        atLimit.Description!.Length.ShouldBe(2048);
    }

    [Fact]
    public async Task GetList_Should_Contain_Created_Projects()
    {
        var created = await CreateProjectAsync(title: "Mobile App");

        var list = await _projectAppService.GetListAsync(new PagedAndSortedResultRequestDto());

        list.TotalCount.ShouldBeGreaterThanOrEqualTo(1);
        list.Items.ShouldContain(p => p.Id == created.Id && p.Title == "Mobile App");
    }

    [Fact]
    public async Task Update_Should_Change_Fields()
    {
        var created = await CreateProjectAsync();

        var updated = await _projectAppService.UpdateAsync(created.Id, new CreateUpdateProjectDto
        {
            Title = "Website Redesign v2",
            Description = "Updated",
            Color = "#445566",
            IntervalMinutes = 15
        });

        updated.Title.ShouldBe("Website Redesign v2");
        updated.IntervalMinutes.ShouldBe(15);
        updated.Color.ShouldBe("#445566");
    }

    [Fact]
    public async Task Delete_Should_Remove_Project_From_Reads()
    {
        var created = await CreateProjectAsync();

        await _projectAppService.DeleteAsync(created.Id);

        await Should.ThrowAsync<EntityNotFoundException>(
            () => _projectAppService.GetAsync(created.Id));
    }

    [Fact]
    public async Task Delete_Should_Remove_The_Projects_Memberships()
    {
        var created = await CreateProjectAsync();
        var other = await CreateProjectAsync(title: "Survivor");

        await _projectAppService.DeleteAsync(created.Id);

        await WithUnitOfWorkAsync(async () =>
        {
            (await _memberRepository.GetListAsync(m => m.ProjectId == created.Id)).ShouldBeEmpty();
            (await _memberRepository.GetListAsync(m => m.ProjectId == other.Id)).ShouldHaveSingleItem();
        });
        (await _projectAppService.GetMyProjectsAsync()).ShouldHaveSingleItem().Id.ShouldBe(other.Id);
    }

    [Fact]
    public async Task MyProjects_Should_Leave_Out_Projects_The_Caller_Is_Only_A_Client_Of()
    {
        var tracked = await SeedProjectWithCallerAsync("Tracked", ProjectMemberRoles.Worker);
        var managed = await SeedProjectWithCallerAsync("Managed", ProjectMemberRoles.Manager);
        var legacyLabel = await SeedProjectWithCallerAsync("Legacy label", "Employee");
        await SeedProjectWithCallerAsync("Client only", ProjectMemberRoles.Client);
        await SeedProjectWithCallerAsync("Client, lower case", "client");
        await SeedProjectWithCallerAsync("Client, padded", " Client ");
        await SeedProjectWithCallerAsync("Not a member");

        var mine = await _projectAppService.GetMyProjectsAsync();

        mine.Select(p => p.Id).ShouldBe(new[] { legacyLabel, managed, tracked }); // ordered by title
    }

    [Fact]
    public async Task Create_Should_Reject_Interval_Outside_Agent_Range()
    {
        // Both desktop agents clamp the capture interval to 5..60 minutes;
        // the server must not accept projects the agents cannot honor.
        await Should.ThrowAsync<AbpValidationException>(
            () => CreateProjectAsync(intervalMinutes: 3));

        await Should.ThrowAsync<AbpValidationException>(
            () => CreateProjectAsync(intervalMinutes: 90));
    }

    [Fact]
    public async Task Create_Should_Default_To_Privacy_First_Capture_Flags()
    {
        var created = await CreateProjectAsync();

        created.AllowScreenshot.ShouldBeTrue();
        created.AllowWebcam.ShouldBeFalse(); // webcam is opt-in
        created.AllowKeyboard.ShouldBeTrue();
        created.AllowMouse.ShouldBeTrue();
        created.AllowActiveWindow.ShouldBeTrue();
        created.AllowRunningPrograms.ShouldBeTrue();
    }

    [Fact]
    public async Task Creator_Should_Automatically_Become_A_Member_And_See_The_Project()
    {
        var created = await CreateProjectAsync(title: "Auto Membership");

        var mine = await _projectAppService.GetMyProjectsAsync();

        mine.ShouldContain(p => p.Id == created.Id && p.Title == "Auto Membership");
        var entry = mine.First(p => p.Id == created.Id);
        entry.IntervalMinutes.ShouldBe(10);
        entry.AllowScreenshot.ShouldBeTrue();
        entry.AllowWebcam.ShouldBeFalse();
        entry.IdleThresholdMinutes.ShouldBe(5);
        entry.BlurScreenshots.ShouldBeFalse();
    }

    [Fact]
    public async Task MyProjects_Should_Push_The_Idle_And_Blur_Policy_To_The_Agents()
    {
        var created = await _projectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = "Blurred Project",
            Color = "#112233",
            IdleThresholdMinutes = 12,
            BlurScreenshots = true
        });

        var entry = (await _projectAppService.GetMyProjectsAsync()).Single(p => p.Id == created.Id);

        entry.IdleThresholdMinutes.ShouldBe(12);
        entry.BlurScreenshots.ShouldBeTrue();
    }

    [Fact]
    public async Task Archive_Should_Hide_The_Project_From_MyProjects()
    {
        var created = await CreateProjectAsync(title: "To Be Archived");

        var archived = await _projectAppService.ArchiveAsync(created.Id);
        archived.IsArchived.ShouldBeTrue();

        var mine = await _projectAppService.GetMyProjectsAsync();
        mine.ShouldNotContain(p => p.Id == created.Id);

        var restored = await _projectAppService.UnarchiveAsync(created.Id);
        restored.IsArchived.ShouldBeFalse();

        mine = await _projectAppService.GetMyProjectsAsync();
        mine.ShouldContain(p => p.Id == created.Id);
    }

    [Fact]
    public async Task Archive_Should_Record_When_The_Project_Was_Archived_And_Unarchive_Should_Clear_It()
    {
        var created = await CreateProjectAsync(title: "Archive Moment");
        var before = DateTime.UtcNow.AddSeconds(-1);

        var archived = await _projectAppService.ArchiveAsync(created.Id);

        archived.ArchivedAt.ShouldNotBeNull();
        archived.ArchivedAt!.Value.ShouldBeGreaterThanOrEqualTo(before);
        archived.ArchivedAt.Value.ShouldBeLessThanOrEqualTo(DateTime.UtcNow.AddSeconds(1));
        (await _projectAppService.GetAsync(created.Id)).ArchivedAt.ShouldBe(archived.ArchivedAt);

        // Archiving again keeps the original moment.
        (await _projectAppService.ArchiveAsync(created.Id)).ArchivedAt.ShouldBe(archived.ArchivedAt);

        var restored = await _projectAppService.UnarchiveAsync(created.Id);
        restored.ArchivedAt.ShouldBeNull();
        (await _projectAppService.GetAsync(created.Id)).ArchivedAt.ShouldBeNull();
    }
}
