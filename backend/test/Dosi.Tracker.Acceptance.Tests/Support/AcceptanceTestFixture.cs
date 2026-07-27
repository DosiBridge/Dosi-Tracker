using System;
using System.Threading.Tasks;
using Dosi.Tracker.EntityFrameworkCore;

namespace Dosi.Tracker.Acceptance.Support;

/// <summary>
/// One live ABP application (with its own fresh in-memory SQLite database and seeded plans) per
/// BDD scenario. Extends the same integration harness the xUnit tests use, and re-exposes its
/// protected helpers publicly so Reqnroll step classes — which live outside the inheritance tree —
/// can resolve services and run units of work. Created and disposed by <see cref="ScenarioLifetimeHooks"/>.
/// </summary>
public sealed class AcceptanceTestFixture : TrackerEntityFrameworkCoreTestBase
{
    public T Resolve<T>() where T : notnull => GetRequiredService<T>();

    public Task WithUow(Func<Task> action) => WithUnitOfWorkAsync(action);

    public Task<TResult> WithUow<TResult>(Func<Task<TResult>> func) => WithUnitOfWorkAsync(func);

    public void Shutdown() => Dispose();
}
