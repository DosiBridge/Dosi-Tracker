using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.Projects;
using Reqnroll;

namespace Dosi.Tracker.Acceptance.Features;

/// <summary>Steps shared across features (project setup). Bindings are global in Reqnroll, so a
/// step is defined exactly once here and reused by any feature that needs it.</summary>
[Binding]
public sealed class CommonSteps
{
    private readonly AcceptanceTestFixture _app;
    private readonly ScenarioState _state;

    public CommonSteps(AcceptanceTestFixture app, ScenarioState state)
    {
        _app = app;
        _state = state;
    }

    [Given(@"I have created a project called ""(.*)""")]
    public async Task GivenIHaveCreatedAProjectCalled(string title)
    {
        var projects = _app.Resolve<IProjectAppService>();
        // CreateAsync auto-enrolls the creator (the acting user) as an Admin member of the project.
        var project = await projects.CreateAsync(new CreateUpdateProjectDto
        {
            Title = title,
            Color = "#006bff",
            IntervalMinutes = 10
        });
        _state.ProjectId = project.Id;
    }
}
