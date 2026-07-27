using System;
using System.Linq;
using System.Threading.Tasks;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class WorkspaceAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IWorkspaceAppService _workspaceAppService;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly ITenantRepository _tenantRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;

    protected WorkspaceAppServiceTests()
    {
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    [Fact]
    public async Task GetAvailablePlans_Should_Return_The_Seeded_Default_Plans()
    {
        var plans = await _workspaceAppService.GetAvailablePlansAsync();

        plans.Items.Count.ShouldBeGreaterThanOrEqualTo(3);
        plans.Items.ShouldContain(p => p.Name == "Free" && p.PricePerUser == 0m && p.MaxSeats == 3);
        plans.Items.ShouldContain(p => p.Name == "Starter" && p.PricePerUser == 6m && p.TrialDays == 14);
        plans.Items.ShouldContain(p => p.Name == "Business" && p.PricePerUser == 12m);
    }

    [Fact]
    public async Task GetCurrentSubscription_Should_Return_Null_When_None_Exists()
    {
        var subscription = await _workspaceAppService.GetCurrentSubscriptionAsync();

        subscription.ShouldBeNull();
    }

    [Fact]
    public async Task GetCurrentSubscription_Should_Return_The_Tenant_Subscription()
    {
        var planId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            // Test host runs without a tenant, so TenantId == null matches CurrentTenant.Id.
            await _subscriptionRepository.InsertAsync(
                new Subscription(_guidGenerator.Create(), null, planId));
        });

        var subscription = await _workspaceAppService.GetCurrentSubscriptionAsync();

        subscription.ShouldNotBeNull();
        subscription.PlanId.ShouldBe(planId);
        subscription.Status.ShouldBe("trialing");
    }

    [Fact]
    public async Task Register_Should_Create_Tenant_Admin_And_Trialing_Subscription()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "acme",
            AdminEmail = "owner@acme.test",
            AdminPassword = "1q2w3E*acme",
            PlanName = "Starter"
        });

        result.TenantId.ShouldNotBe(Guid.Empty);
        result.Name.ShouldBe("acme");

        await WithUnitOfWorkAsync(async () =>
        {
            var tenant = await _tenantRepository.FindAsync(result.TenantId);
            tenant.ShouldNotBeNull();
            tenant.Name.ShouldBe("acme");
        });

        // The tenant got its admin user and a trialing Starter subscription.
        using (_currentTenant.Change(result.TenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var userRepository = GetRequiredService<IIdentityUserRepository>();
                var users = await userRepository.GetListAsync();
                users.ShouldContain(u => u.Email == "owner@acme.test");

                var subscription = await _subscriptionRepository.FirstOrDefaultAsync(
                    s => s.TenantId == result.TenantId);
                subscription.ShouldNotBeNull();
                subscription.Status.ShouldBe("trialing");
                subscription.TrialEndsAt.ShouldNotBeNull();
            });
        }
    }

    [Fact]
    public async Task Register_Should_Activate_Immediately_On_The_Free_Plan()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "freeco",
            AdminEmail = "owner@freeco.test",
            AdminPassword = "1q2w3E*freeco"
        });

        using (_currentTenant.Change(result.TenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var subscription = await _subscriptionRepository.FirstOrDefaultAsync(
                    s => s.TenantId == result.TenantId);
                subscription.ShouldNotBeNull();
                subscription.Status.ShouldBe("active"); // nothing to trial on Free
                subscription.TrialEndsAt.ShouldBeNull();
            });
        }
    }

    [Fact]
    public async Task Register_Should_Reject_Unknown_Plans()
    {
        var exception = await Should.ThrowAsync<BusinessException>(
            () => _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
            {
                Name = "nope",
                AdminEmail = "owner@nope.test",
                AdminPassword = "1q2w3E*nope",
                PlanName = "Platinum"
            }));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
    }
}
