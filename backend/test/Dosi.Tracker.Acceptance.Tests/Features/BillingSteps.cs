using System;
using System.Globalization;
using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Reqnroll;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;

namespace Dosi.Tracker.Acceptance.Features;

[Binding]
public sealed class BillingSteps
{
    private readonly AcceptanceTestFixture _app;
    private readonly ScenarioState _state;

    public BillingSteps(AcceptanceTestFixture app, ScenarioState state)
    {
        _app = app;
        _state = state;
    }

    [Given(@"the workspace has a ""(.*)"" subscription")]
    public async Task GivenTheWorkspaceHasASubscription(string planName)
    {
        await _app.WithUow(async () =>
        {
            var plans = _app.Resolve<IRepository<Plan, Guid>>();
            var guids = _app.Resolve<IGuidGenerator>();
            var plan = await plans.FirstAsync(p => p.Name == planName);
            _state.Subscription = new Subscription(guids.Create(), null, plan.Id);
        });
    }

    [Given(@"(\d+) people occupy seats")]
    public async Task GivenPeopleOccupySeats(int count)
    {
        await _app.WithUow(async () =>
        {
            var projects = _app.Resolve<IRepository<Project, Guid>>();
            var members = _app.Resolve<IRepository<ProjectMember, Guid>>();
            var guids = _app.Resolve<IGuidGenerator>();
            var projectId = guids.Create();
            await projects.InsertAsync(new Project(projectId, null, "Billable Project"));
            for (var i = 0; i < count; i++)
            {
                await members.InsertAsync(new ProjectMember(guids.Create(), null, projectId, Guid.NewGuid()));
            }
        });
    }

    [When(@"the monthly invoice is generated")]
    public async Task WhenTheMonthlyInvoiceIsGenerated()
    {
        var subscription = (Subscription)_state.Subscription!;
        _state.LastInvoice = await _app.WithUow(
            () => _app.Resolve<InvoiceGenerator>().GenerateForCurrentMonthAsync(subscription));
    }

    [Then(@"an invoice for (.*) is created")]
    public void ThenAnInvoiceForIsCreated(string amount)
    {
        _state.LastInvoice.ShouldNotBeNull();
        ((Invoice)_state.LastInvoice!).Amount.ShouldBe(decimal.Parse(amount, CultureInfo.InvariantCulture));
    }

    [Then(@"no invoice is created")]
    public void ThenNoInvoiceIsCreated()
    {
        _state.LastInvoice.ShouldBeNull();
    }

    [Then(@"regenerating in the same month does not bill again")]
    public async Task ThenRegeneratingInTheSameMonthDoesNotBillAgain()
    {
        var subscription = (Subscription)_state.Subscription!;
        var second = await _app.WithUow(
            () => _app.Resolve<InvoiceGenerator>().GenerateForCurrentMonthAsync(subscription));
        second.ShouldBeNull();

        await _app.WithUow(async () =>
        {
            var invoices = _app.Resolve<IRepository<Invoice, Guid>>();
            (await invoices.GetCountAsync()).ShouldBe(1);
        });
    }
}
