using System;
using System.Threading.Tasks;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.SaaS;

/// <summary>Marks trialing subscriptions whose trial has ended as past_due (hourly).</summary>
public class TrialExpiryWorker : AsyncPeriodicBackgroundWorkerBase
{
    public TrialExpiryWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromHours(1).TotalMilliseconds;
    }

    protected override async Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        var serviceProvider = workerContext.ServiceProvider;
        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var clock = serviceProvider.GetRequiredService<IClock>();
        var subscriptionRepository = serviceProvider.GetRequiredService<IRepository<Subscription, Guid>>();

        using var uow = uowManager.Begin(requiresNew: true);
        using (dataFilter.Disable<IMultiTenant>())
        {
            var now = clock.Now;
            var expired = await subscriptionRepository.GetListAsync(
                s => s.Status == "trialing" && s.TrialEndsAt != null && s.TrialEndsAt < now);

            foreach (var subscription in expired)
            {
                subscription.Status = "past_due";
                await subscriptionRepository.UpdateAsync(subscription);
                Logger.LogInformation(
                    "Subscription {SubscriptionId} (tenant {TenantId}) trial expired; marked past_due.",
                    subscription.Id, subscription.TenantId);
            }
        }

        await uow.CompleteAsync();
    }
}
