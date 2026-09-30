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
/// idempotent — at most one invoice per tenant per calendar month). Only <c>active</c> subscriptions
/// are billable: a <c>trialing</c> workspace is never invoiced during its trial.</summary>
public class InvoiceGenerationWorker : AsyncPeriodicBackgroundWorkerBase
{
    public InvoiceGenerationWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromDays(1).TotalMilliseconds;
    }

    /// <summary>Subscription status that is invoiced; every other status (trialing, past_due, …) is skipped.</summary>
    public const string BillableStatus = "active";

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return GenerateInvoicesAsync(workerContext.ServiceProvider);
    }

    /// <summary>One sweep over every tenant's billable subscription (the body of the daily run).</summary>
    public virtual async Task GenerateInvoicesAsync(IServiceProvider serviceProvider)
    {
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
                s => s.Status == BillableStatus)).ToArray();
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
