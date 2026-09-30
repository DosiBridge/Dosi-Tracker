using System;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities.Events;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.EventBus;
using Volo.Abp.Guids;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Volo.Abp.Timing;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// Every tenant needs a subscription (seat limits, billing, the platform console's plan counts).
/// Workspace sign-up creates one itself; tenants created from the host console through ABP's tenant
/// endpoint do not — this gives those a subscription on the default (Free) plan.
/// Local entity events run inside the creating unit of work, after SaveChanges, so a subscription
/// inserted by the same request (sign-up) is visible here and is left alone.
/// </summary>
public class DefaultSubscriptionForNewTenantHandler :
    ILocalEventHandler<EntityCreatedEventData<Tenant>>,
    ITransientDependency
{
    public ILogger<DefaultSubscriptionForNewTenantHandler> Logger { get; set; }

    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly ICurrentTenant _currentTenant;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public DefaultSubscriptionForNewTenantHandler(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        ICurrentTenant currentTenant,
        IGuidGenerator guidGenerator,
        IClock clock)
    {
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _currentTenant = currentTenant;
        _guidGenerator = guidGenerator;
        _clock = clock;
        Logger = NullLogger<DefaultSubscriptionForNewTenantHandler>.Instance;
    }

    public virtual async Task HandleEventAsync(EntityCreatedEventData<Tenant> eventData)
    {
        var tenantId = eventData.Entity.Id;

        // Plans are host-level (not IMultiTenant), so they are readable from any context.
        var plan = await _planRepository.FirstOrDefaultAsync(p => p.Name == Plan.FreePlanName)
                   ?? (await _planRepository.GetListAsync()).OrderBy(p => p.PricePerUser).FirstOrDefault();
        if (plan == null)
        {
            Logger.LogWarning("No plans exist; tenant {TenantId} was created without a subscription.", tenantId);
            return;
        }

        using (_currentTenant.Change(tenantId))
        {
            if (await _subscriptionRepository.AnyAsync(s => s.TenantId == tenantId))
            {
                return; // Workspace sign-up already chose a plan.
            }

            var subscription = new Subscription(_guidGenerator.Create(), tenantId, plan.Id);
            if (plan.TrialDays > 0)
            {
                subscription.TrialEndsAt = _clock.Now.AddDays(plan.TrialDays);
            }
            else
            {
                subscription.Status = "active";
            }

            await _subscriptionRepository.InsertAsync(subscription);
            Logger.LogInformation("Tenant {TenantId} subscribed to the default '{Plan}' plan.", tenantId, plan.Name);
        }
    }
}
