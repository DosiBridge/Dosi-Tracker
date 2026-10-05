using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// Hourly subscription sweep, also run once at start-up so restarts never postpone it (see
/// <see cref="SubscriptionManager.ReviewAsync"/>): reminds owners before a trial ends, marks ended trials past_due,
/// and marks active workspaces past_due when an invoice goes unpaid past its due date. Deleted workspaces are
/// skipped, and sweeps do not overlap (named lock, held across replicas when a distributed lock provider is
/// configured).
/// </summary>
public class TrialExpiryWorker : AsyncPeriodicBackgroundWorkerBase
{
    /// <summary>Name of the lock that keeps two sweeps from running at the same time.</summary>
    public const string LockName = "Tracker:SubscriptionSweep";

    public TrialExpiryWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromHours(1).TotalMilliseconds;
        Timer.RunOnStart = true;
    }

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return CheckSubscriptionsAsync(workerContext.ServiceProvider);
    }

    /// <summary>One sweep over every workspace's subscription that may need attention (the body of the hourly run).</summary>
    public virtual async Task<SubscriptionSweepResult> CheckSubscriptionsAsync(IServiceProvider serviceProvider)
    {
        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null ? null : await distributedLock.TryAcquireAsync(LockName);
        if (distributedLock != null && sweepLock == null)
        {
            Logger.LogInformation("Another subscription sweep is running; this one is skipped.");
            return new SubscriptionSweepResult { Skipped = true };
        }

        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var currentTenant = serviceProvider.GetRequiredService<ICurrentTenant>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var subscriptionRepository = serviceProvider.GetRequiredService<IRepository<Subscription, Guid>>();
        var invoiceRepository = serviceProvider.GetRequiredService<IRepository<Invoice, Guid>>();
        var tenantRepository = serviceProvider.GetRequiredService<IRepository<Tenant, Guid>>();
        var subscriptionManager = serviceProvider.GetRequiredService<SubscriptionManager>();
        var now = serviceProvider.GetRequiredService<IClock>().Now;

        // Only subscriptions with something to do: trials ending within the reminder window (or ended), and
        // active workspaces with an unpaid invoice past its due date.
        (Guid Id, Guid? TenantId)[] candidates;
        using (var readUow = uowManager.Begin(requiresNew: true))
        using (dataFilter.Disable<IMultiTenant>())
        {
            var reminderHorizon = now.AddDays(Subscription.TrialReminderDays);
            var subscriptions = await subscriptionRepository.GetQueryableAsync();
            var invoices = await invoiceRepository.GetQueryableAsync();
            var tenants = await tenantRepository.GetQueryableAsync(); // soft-delete filtered: deleted workspaces drop out

            var rows = await asyncExecuter.ToListAsync(subscriptions
                .Where(s => tenants.Any(t => t.Id == s.TenantId))
                .Where(s =>
                    (s.Status == Subscription.TrialingStatus && s.TrialEndsAt != null && s.TrialEndsAt <= reminderHorizon) ||
                    (s.Status == Subscription.ActiveStatus && invoices.Any(i =>
                        i.TenantId == s.TenantId && i.Status != Invoice.PaidStatus && i.DueDate < now)))
                .Select(s => new { s.Id, s.TenantId }));
            candidates = rows.Select(r => (r.Id, r.TenantId)).ToArray();

            await readUow.CompleteAsync();
        }

        // One UoW per workspace so one failure neither rolls back nor stops the others.
        var result = new SubscriptionSweepResult();
        foreach (var (subscriptionId, tenantId) in candidates)
        {
            try
            {
                using var uow = uowManager.Begin(requiresNew: true);
                using (currentTenant.Change(tenantId))
                {
                    var subscription = await subscriptionRepository.GetAsync(subscriptionId);
                    result.Add(await subscriptionManager.ReviewAsync(subscription));
                }

                await uow.CompleteAsync();
            }
            catch (Exception ex)
            {
                result.Failed++;
                Logger.LogError(ex,
                    "Failed to review subscription {SubscriptionId} of tenant {TenantId}; continuing with the rest.",
                    subscriptionId, tenantId);
            }
        }

        return result;
    }
}
