using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Reqnroll;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Acceptance.Features;

[Binding]
public sealed class ActivityTrackingSteps
{
    /// <summary>A recent past day (UTC midnight): uploads are accepted only within the 45-day window (contract §1).</summary>
    private static readonly DateTime BaseDay = DateTime.UtcNow.Date.AddDays(-7);

    private readonly AcceptanceTestFixture _app;
    private readonly ScenarioState _state;

    public ActivityTrackingSteps(AcceptanceTestFixture app, ScenarioState state)
    {
        _app = app;
        _state = state;
    }

    private static CreateActivityDto Build(Guid projectId, Guid clientId, DateTime start) => new()
    {
        ProjectId = projectId,
        ClientActivityId = clientId,
        DeviceId = Guid.Parse("5d0c1f6a-3b8e-4a51-9c2e-7f40a1b2c3d4"),
        StartedAt = start,
        EndedAt = start.AddMinutes(10),
        Productivity = 80,
        MouseClicks = 120,
        KeyboardHits = 450,
        Description = "Working on the tracker",
        ActiveWindows = new List<WindowInfoDto> { new() { AppName = "code.exe", WindowTitle = "ActivityAppService.cs" } },
        RunningPrograms = new List<WindowInfoDto> { new() { AppName = "chrome.exe", WindowTitle = "Dosi-Tracker" } }
    };

    [Given(@"a project exists that I do not belong to")]
    public async Task GivenAProjectExistsThatIDoNotBelongTo()
    {
        await _app.WithUow(async () =>
        {
            var projects = _app.Resolve<IRepository<Project, Guid>>();
            var id = Guid.NewGuid();
            // Inserted directly, with no ProjectMember for the current user.
            await projects.InsertAsync(new Project(id, TrackerTestConsts.WorkspaceId, "Foreign Project"));
            _state.ProjectId = id;
        });
    }

    [When(@"I submit a tracked activity for the project")]
    public async Task WhenISubmitATrackedActivityForTheProject()
    {
        var activities = _app.Resolve<IActivityAppService>();
        var result = await activities.CreateAsync(Build(_state.ProjectId, Guid.NewGuid(), BaseDay.AddHours(10)));
        _state.LastResult = result;
        _state.ActivityIds.Add(result.Id);
    }

    [When(@"I submit a tracked activity for that other project")]
    public async Task WhenISubmitATrackedActivityForThatOtherProject()
    {
        var activities = _app.Resolve<IActivityAppService>();
        try
        {
            await activities.CreateAsync(Build(_state.ProjectId, Guid.NewGuid(), BaseDay.AddHours(10)));
        }
        catch (Exception ex)
        {
            _state.LastException = ex;
        }
    }

    [When(@"I submit the same activity twice")]
    public async Task WhenISubmitTheSameActivityTwice()
    {
        var activities = _app.Resolve<IActivityAppService>();
        var clientId = Guid.NewGuid(); // identical client id => same tracked block
        var first = await activities.CreateAsync(Build(_state.ProjectId, clientId, BaseDay.AddHours(10)));
        var second = await activities.CreateAsync(Build(_state.ProjectId, clientId, BaseDay.AddHours(10)));
        _state.ActivityIds.Add(first.Id);
        _state.ActivityIds.Add(second.Id);
    }

    [When(@"I submit tracked activities starting at hours (\d+), (\d+) and (\d+)")]
    public async Task WhenISubmitTrackedActivitiesStartingAtHours(int h1, int h2, int h3)
    {
        var activities = _app.Resolve<IActivityAppService>();
        foreach (var hour in new[] { h1, h2, h3 })
        {
            await activities.CreateAsync(Build(_state.ProjectId, Guid.NewGuid(), BaseDay.AddHours(hour)));
        }
    }

    [Then(@"the activity is recorded and attributed to me")]
    public void ThenTheActivityIsRecordedAndAttributedToMe()
    {
        var activity = _state.Result<ActivityDto>();
        activity.Id.ShouldNotBe(Guid.Empty);
        activity.UserId.ShouldBe(ScenarioState.CurrentUserId);
        activity.ProjectId.ShouldBe(_state.ProjectId);
    }

    [Then(@"tracking is refused as unauthorized")]
    public void ThenTrackingIsRefusedAsUnauthorized()
    {
        // Refused with the Tracker:NotProjectMember business error (HTTP 403).
        _state.LastException.ShouldBeOfType<BusinessException>()
            .Code.ShouldBe(TrackerDomainErrorCodes.NotProjectMember);
    }

    [Then(@"only one activity is stored for the project")]
    public async Task ThenOnlyOneActivityIsStoredForTheProject()
    {
        _state.ActivityIds.Distinct().Count().ShouldBe(1); // the re-upload returned the same activity

        await _app.WithUow(async () =>
        {
            var repository = _app.Resolve<IRepository<Activity, Guid>>();
            var count = await repository.CountAsync(a => a.ProjectId == _state.ProjectId);
            count.ShouldBe(1);
        });
    }

    [Then(@"the activities are listed newest first")]
    public async Task ThenTheActivitiesAreListedNewestFirst()
    {
        var activities = _app.Resolve<IActivityAppService>();
        var page = await activities.GetListAsync(new GetActivitiesInput
        {
            ProjectId = _state.ProjectId,
            MaxResultCount = 10
        });

        page.TotalCount.ShouldBe(3);
        page.Items[0].StartedAt.ShouldBe(BaseDay.AddHours(10));
        page.Items[1].StartedAt.ShouldBe(BaseDay.AddHours(9));
        page.Items[2].StartedAt.ShouldBe(BaseDay.AddHours(8));
    }
}
