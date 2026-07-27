using System;
using System.Linq;
using System.Threading.Tasks;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Modularity;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.Projects;

public abstract class ProjectAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IProjectAppService _projectAppService;

    protected ProjectAppServiceTests()
    {
        _projectAppService = GetRequiredService<IProjectAppService>();
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
}
