using System;
using System.Threading.Tasks;
using Dosi.Tracker.Acceptance.Support;
using Dosi.Tracker.SaaS;
using Reqnroll;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;

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
                Name = name,
                AdminEmail = adminEmail,
                AdminPassword = "1q2w3E*" + name,
                PlanName = plan
            });
            _state.LastResult = result;
            _state.TenantId = result.TenantId;
            _state.WorkspaceName = result.Name;
        }
        catch (Exception ex)
        {
            _state.LastException = ex;
        }
    }

    [Then(@"the workspace is created with its own tenant")]
    public void ThenTheWorkspaceIsCreatedWithItsOwnTenant()
    {
        _state.LastException.ShouldBeNull();
        _state.TenantId.ShouldNotBe(Guid.Empty);
    }

    [Then(@"the tenant has an administrator ""(.*)""")]
    public async Task ThenTheTenantHasAnAdministrator(string email)
    {
        var currentTenant = _app.Resolve<ICurrentTenant>();
        using (currentTenant.Change(_state.TenantId))
        {
            await _app.WithUow(async () =>
            {
                var users = _app.Resolve<IIdentityUserRepository>();
                var all = await users.GetListAsync();
                all.ShouldContain(u => u.Email == email);
            });
        }
    }

    [Then(@"the subscription is ""(.*)""")]
    public async Task ThenTheSubscriptionIs(string status)
    {
        var currentTenant = _app.Resolve<ICurrentTenant>();
        using (currentTenant.Change(_state.TenantId))
        {
            await _app.WithUow(async () =>
            {
                var subscriptions = _app.Resolve<IRepository<Subscription, Guid>>();
                var subscription = await subscriptions.FirstOrDefaultAsync(s => s.TenantId == _state.TenantId);
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
