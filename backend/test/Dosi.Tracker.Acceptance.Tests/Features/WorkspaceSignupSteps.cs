using System;
using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.SaaS;
using Reqnroll;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Acceptance.Features;

[Binding]
public sealed class WorkspaceSignupSteps
{
    private readonly AcceptanceTestFixture _app;
    private readonly ScenarioState _state;

    public WorkspaceSignupSteps(AcceptanceTestFixture app, ScenarioState state)
    {
        _app = app;
        _state = state;
    }

    [When(@"I register the workspace ""(.*)"" for admin ""(.*)"" on the ""(.*)"" plan")]
    public async Task WhenIRegisterTheWorkspace(string name, string adminEmail, string plan)
    {
        var workspaces = _app.Resolve<IWorkspaceAppService>();
        try
        {
            var result = await workspaces.RegisterAsync(new RegisterWorkspaceDto
            {
                WorkspaceName = name,
                Email = adminEmail,
                Password = "1q2w3E*" + name,
                PlanName = plan
            });
            _state.LastResult = result;
            _state.WorkspaceId = result.WorkspaceId;
            _state.WorkspaceName = result.WorkspaceName;
        }
        catch (Exception ex)
        {
            _state.LastException = ex;
        }
    }

    [Then(@"the workspace is created")]
    public void ThenTheWorkspaceIsCreated()
    {
        _state.LastException.ShouldBeNull();
        _state.WorkspaceId.ShouldNotBe(Guid.Empty);
    }

    [Then(@"""(.*)"" owns the workspace")]
    public async Task ThenOwnsTheWorkspace(string email)
    {
        await _app.WithUow(async () =>
        {
            var user = await _app.Resolve<IdentityUserManager>().FindByEmailAsync(email);
            user.ShouldNotBeNull();
            var member = await _app.Resolve<WorkspaceMembershipManager>().FindActiveAsync(_state.WorkspaceId, user.Id);
            member.ShouldNotBeNull().Role.ShouldBe(WorkspaceRoles.Owner);
        });
    }

    [Then(@"the subscription is ""(.*)""")]
    public async Task ThenTheSubscriptionIs(string status)
    {
        var currentTenant = _app.Resolve<ICurrentWorkspace>();
        using (currentTenant.Change(_state.WorkspaceId))
        {
            await _app.WithUow(async () =>
            {
                var subscriptions = _app.Resolve<IRepository<Subscription, Guid>>();
                var subscription = await subscriptions.FirstOrDefaultAsync(s => s.WorkspaceId == _state.WorkspaceId);
                subscription.ShouldNotBeNull();
                subscription.Status.ShouldBe(status);
            });
        }
    }

    [Then(@"registration is rejected because the plan does not exist")]
    public void ThenRegistrationIsRejectedBecauseThePlanDoesNotExist()
    {
        _state.LastException.ShouldBeOfType<BusinessException>()
            .Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
    }
}
