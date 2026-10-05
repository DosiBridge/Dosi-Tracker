using System;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities.Events;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.EventBus;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// A deleted workspace is no longer a customer: its subscription is canceled and soft-deleted, so it is never
/// invoiced again, drops out of the platform console's revenue, seat and subscriber counts, and no longer keeps
/// its plan from being deleted. Its invoices stay as billing history. Runs inside the deleting unit of work.
/// </summary>
public class CancelSubscriptionOnTenantDeletedHandler :
    ILocalEventHandler<EntityDeletedEventData<Tenant>>,
    ITransientDependency
{
    public ILogger<CancelSubscriptionOnTenantDeletedHandler> Logger { get; set; }

    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly ICurrentTenant _currentTenant;

    public CancelSubscriptionOnTenantDeletedHandler(
        IRepository<Subscription, Guid> subscriptionRepository,
        ICurrentTenant currentTenant)
    {
        _subscriptionRepository = subscriptionRepository;
        _currentTenant = currentTenant;
        Logger = NullLogger<CancelSubscriptionOnTenantDeletedHandler>.Instance;
    }

    public virtual async Task HandleEventAsync(EntityDeletedEventData<Tenant> eventData)
    {
        var tenantId = eventData.Entity.Id;

        using (_currentTenant.Change(tenantId))
        {
            var subscriptions = await _subscriptionRepository.GetListAsync(s => s.TenantId == tenantId);
            foreach (var subscription in subscriptions)
            {
                // Saved before the delete: soft-deleting reloads the entity, which would drop the new status.
                subscription.Cancel();
                await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);
                await _subscriptionRepository.DeleteAsync(subscription);

                Logger.LogInformation(
                    "Workspace {TenantId} was deleted; subscription {SubscriptionId} canceled.",
                    tenantId, subscription.Id);
            }
        }
    }
}
