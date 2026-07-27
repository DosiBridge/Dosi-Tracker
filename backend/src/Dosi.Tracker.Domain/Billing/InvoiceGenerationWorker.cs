using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Threading;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Billing;

/// <summary>Generates the monthly per-seat invoice for every billable subscription (daily sweep;
/// idempotent — at most one invoice per tenant per calendar month).</summary>
public class InvoiceGenerationWorker : AsyncPeriodicBackgroundWorkerBase
{
    public InvoiceGenerationWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromDays(1).TotalMilliseconds;
    }

    protected override async Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        var serviceProvider = workerContext.ServiceProvider;
        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var currentTenant = serviceProvider.GetRequiredService<ICurrentTenant>();
        var subscriptionRepository = serviceProvider.GetRequiredService<IRepository<Subscription, Guid>>();
        var invoiceGenerator = serviceProvider.GetRequiredService<InvoiceGenerator>();

        Subscription[] billable;
        using (var readUow = uowManager.Begin(requiresNew: true))
        using (dataFilter.Disable<IMultiTenant>())
        {
            billable = (await subscriptionRepository.GetListAsync(
                s => s.Status == "active" || s.Status == "trialing")).ToArray();
            await readUow.CompleteAsync();
        }

        // One UoW per tenant so a single tenant's failure can't roll back other tenants'
        // invoices or abort the rest of the sweep.
        foreach (var subscription in billable)
        {
            try
            {
                using var uow = uowManager.Begin(requiresNew: true);
                using (currentTenant.Change(subscription.TenantId))
                {
                    var invoice = await invoiceGenerator.GenerateForCurrentMonthAsync(subscription);
                    if (invoice != null)
                    {
                        Logger.LogInformation(
                            "Invoice {InvoiceId} ({Amount}) generated for tenant {TenantId}.",
                            invoice.Id, invoice.Amount, subscription.TenantId);
                    }
                }

                await uow.CompleteAsync();
            }
            catch (Exception ex)
            {
                Logger.LogError(ex,
                    "Failed to generate the monthly invoice for tenant {TenantId}; continuing with the rest.",
                    subscription.TenantId);
            }
        }
    }
}
