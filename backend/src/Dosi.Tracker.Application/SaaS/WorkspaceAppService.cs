using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.TenantManagement;

namespace Dosi.Tracker.SaaS;

[Authorize]
public class WorkspaceAppService : TrackerAppService, IWorkspaceAppService
{
    public const string DefaultPlanName = "Free";

    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;
    private readonly IDataSeeder _dataSeeder;

    public WorkspaceAppService(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        ITenantManager tenantManager,
        ITenantRepository tenantRepository,
        IDataSeeder dataSeeder)
    {
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _tenantManager = tenantManager;
        _tenantRepository = tenantRepository;
        _dataSeeder = dataSeeder;
    }

    public async Task<SubscriptionDto> GetCurrentSubscriptionAsync()
    {
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.TenantId == CurrentTenant.Id);
        if (subscription == null) return null;

        return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
    }

    public async Task<ListResultDto<PlanDto>> GetAvailablePlansAsync()
    {
        var plans = await _planRepository.GetListAsync();
        return new ListResultDto<PlanDto>(
            ObjectMapper.Map<List<Plan>, List<PlanDto>>(plans)
        );
    }

    [AllowAnonymous]
    public async Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input)
    {
        var planName = input.PlanName.IsNullOrWhiteSpace() ? DefaultPlanName : input.PlanName!;
        var plan = await _planRepository.FirstOrDefaultAsync(p => p.Name == planName);
        if (plan == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                .WithData("planName", planName);
        }

        var tenant = await _tenantManager.CreateAsync(input.Name);
        await _tenantRepository.InsertAsync(tenant, autoSave: true);

        using (CurrentTenant.Change(tenant.Id, tenant.Name))
        {
            // Seeds the tenant's admin user (Identity), admin role and its permissions.
            await _dataSeeder.SeedAsync(new DataSeedContext(tenant.Id)
                .WithProperty(IdentityDataSeedContributor.AdminEmailPropertyName, input.AdminEmail)
                .WithProperty(IdentityDataSeedContributor.AdminPasswordPropertyName, input.AdminPassword));

            var subscription = new Subscription(GuidGenerator.Create(), tenant.Id, plan.Id);
            if (plan.TrialDays > 0)
            {
                subscription.TrialEndsAt = Clock.Now.AddDays(plan.TrialDays);
            }
            else
            {
                // Free plans have nothing to trial.
                subscription.Status = "active";
            }

            await _subscriptionRepository.InsertAsync(subscription);
        }

        return new WorkspaceRegistrationResultDto
        {
            TenantId = tenant.Id,
            Name = tenant.Name
        };
    }
}
