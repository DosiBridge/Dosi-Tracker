using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Reqnroll;
using Reqnroll.BoDi;
using Shouldly;
using Volo.Abp.Identity;

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

    /// <summary>
    /// The acting user (<see cref="ScenarioState.CurrentUserId"/>) is the workspace owner, with the account a real owner
    /// has: an active user in the Identity "admin" role. Seats, for one, are only held by people with an account.
    /// </summary>
    [BeforeScenario(Order = 1)]
    public async Task CreateTheActingOwnerAsync()
    {
        var app = _container.Resolve<AcceptanceTestFixture>();
        await app.WithUow(async () =>
        {
            var userManager = app.Resolve<IdentityUserManager>();
            var owner = new IdentityUser(ScenarioState.CurrentUserId, "owner", "owner@tracker.test");
            (await userManager.CreateAsync(owner)).Succeeded.ShouldBeTrue();
            (await userManager.AddToRoleAsync(owner, TrackerRoles.Admin)).Succeeded.ShouldBeTrue();
        });
    }

    [AfterScenario(Order = 0)]
    public void StopApplication()
    {
        _container.Resolve<AcceptanceTestFixture>().Shutdown();
    }
}
