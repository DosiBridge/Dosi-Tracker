using Reqnroll;
using Reqnroll.BoDi;

namespace Dosi.Tracker.Acceptance.Support;

/// <summary>
/// Boots a fresh ABP application for each scenario and registers it (plus a blank
/// <see cref="ScenarioState"/>) in the Reqnroll scenario container, so every step class receives the
/// same instances by constructor injection. Tears the application down afterwards.
/// </summary>
[Binding]
public sealed class ScenarioLifetimeHooks
{
    private readonly IObjectContainer _container;

    public ScenarioLifetimeHooks(IObjectContainer container)
    {
        _container = container;
    }

    [BeforeScenario(Order = 0)]
    public void StartApplication()
    {
        _container.RegisterInstanceAs(new AcceptanceTestFixture());
        _container.RegisterInstanceAs(new ScenarioState());
    }

    [AfterScenario(Order = 0)]
    public void StopApplication()
    {
        _container.Resolve<AcceptanceTestFixture>().Shutdown();
    }
}
