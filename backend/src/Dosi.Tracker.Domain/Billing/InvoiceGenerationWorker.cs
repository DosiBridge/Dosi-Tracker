using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Billing;

/// <summary>
/// Generates this month's per-seat invoice for every billable subscription. Runs once at start-up and then hourly,
/// so frequent restarts never postpone billing; the sweep is idempotent (at most one invoice per workspace and
/// billing period). Only <c>active</c> subscriptions of existing, unarchived workspaces are billable: a trial, a
/// past_due or canceled subscription and an archived or deleted workspace are never invoiced, and nothing is while
/// billing is off (<see cref="BillingOptions"/>; the host does not even start the worker then). Sweeps do not
/// overlap (named lock, held across replicas when a distributed lock provider is configured), and the unique index on
/// (workspace, billing period) settles any race that remains: the losing insert is recognized as "already invoiced",
/// not as a failure.
/// </summary>
public class InvoiceGenerationWorker : AsyncPeriodicBackgroundWorkerBase
{
    /// <summary>Name of the lock that keeps two sweeps from running at the same time.</summary>
    public const string LockName = "Tracker:InvoiceSweep";

    public InvoiceGenerationWorker(
        AbpAsyncTimer timer,
        IServiceScopeFactory serviceScopeFactory)
        : base(timer, serviceScopeFactory)
    {
        Timer.Period = (int)TimeSpan.FromHours(1).TotalMilliseconds;
        Timer.RunOnStart = true;
    }

    /// <summary>Subscription status that is invoiced; every other status (trialing, past_due, …) is skipped.</summary>
    public const string BillableStatus = Subscription.ActiveStatus;

    protected override Task DoWorkAsync(PeriodicBackgroundWorkerContext workerContext)
    {
        return GenerateInvoicesAsync(workerContext.ServiceProvider);
    }

    /// <summary>One sweep over every workspace's billable subscription (the body of the hourly run).</summary>
    public virtual async Task<InvoiceSweepResult> GenerateInvoicesAsync(IServiceProvider serviceProvider)
    {
        if (!serviceProvider.GetRequiredService<IOptions<BillingOptions>>().Value.Enabled)
        {
            return new InvoiceSweepResult { BillingDisabled = true };
        }

        var distributedLock = serviceProvider.GetService<IAbpDistributedLock>();
        await using var sweepLock = distributedLock == null ? null : await distributedLock.TryAcquireAsync(LockName);
        if (distributedLock != null && sweepLock == null)
        {
            Logger.LogInformation("Another invoice sweep is running; this one is skipped.");
            return new InvoiceSweepResult { Skipped = true };
        }

        var uowManager = serviceProvider.GetRequiredService<IUnitOfWorkManager>();
        var dataFilter = serviceProvider.GetRequiredService<IDataFilter>();
        var currentWorkspace = serviceProvider.GetRequiredService<ICurrentWorkspace>();
        var asyncExecuter = serviceProvider.GetRequiredService<IAsyncQueryableExecuter>();
        var subscriptionRepository = serviceProvider.GetRequiredService<IRepository<Subscription, Guid>>();
        var workspaceRepository = serviceProvider.GetRequiredService<IRepository<Workspace, Guid>>();
        var invoiceGenerator = serviceProvider.GetRequiredService<InvoiceGenerator>();

        // One period for the whole sweep, even if it runs across midnight at the end of a month.
        var periodStart = Invoice.GetBillingPeriodStart(serviceProvider.GetRequiredService<IClock>().Now);

        Subscription[] billable;
        using (var readUow = uowManager.Begin(requiresNew: true))
        using (dataFilter.Disable<IWorkspaceScoped>())
        {
            var subscriptions = await subscriptionRepository.GetQueryableAsync();
            var workspaces = await workspaceRepository.GetQueryableAsync();
            billable = (await asyncExecuter.ToListAsync(subscriptions.Where(s =>
                s.Status == BillableStatus && workspaces.Any(w => w.Id == s.WorkspaceId && !w.IsArchived)))).ToArray();
            await readUow.CompleteAsync();
        }

        // One UoW per workspace so a single workspace's failure can't roll back other workspaces'
        // invoices or abort the rest of the sweep.
        var result = new InvoiceSweepResult();
        foreach (var subscription in billable)
        {
            try
            {
                Invoice? invoice;
                using (var uow = uowManager.Begin(requiresNew: true))
                {
                    using (currentWorkspace.Change(subscription.WorkspaceId))
                    {
                        invoice = await invoiceGenerator.GenerateForPeriodAsync(subscription, periodStart);
                    }

                    await uow.CompleteAsync();
                }

                if (invoice != null)
                {
                    result.Generated++;
                    Logger.LogInformation(
                        "Invoice {InvoiceId} ({Amount}) generated for workspace {WorkspaceId}.",
                        invoice.Id, invoice.Amount, subscription.WorkspaceId);
                }
            }
            catch (Exception ex)
            {
                if (await IsInvoicedAsync(uowManager, currentWorkspace, invoiceGenerator, subscription, periodStart))
                {
                    // Another sweep inserted this period's invoice between our check and our insert.
                    result.AlreadyInvoiced++;
                    Logger.LogInformation(
                        "Workspace {WorkspaceId} was invoiced for {Period:yyyy-MM} by another sweep.",
                        subscription.WorkspaceId, periodStart);
                    continue;
                }

                result.Failed++;
                Logger.LogError(ex,
                    "Failed to generate the monthly invoice for workspace {WorkspaceId}; continuing with the rest.",
                    subscription.WorkspaceId);
            }
        }

        return result;
    }

    private async Task<bool> IsInvoicedAsync(
        IUnitOfWorkManager uowManager,
        ICurrentWorkspace currentWorkspace,
        InvoiceGenerator invoiceGenerator,
        Subscription subscription,
        DateTime periodStart)
    {
        try
        {
            // A fresh unit of work: the failed one's database context (and transaction) is unusable.
            using var uow = uowManager.Begin(requiresNew: true);
            bool invoiced;
            using (currentWorkspace.Change(subscription.WorkspaceId))
            {
                invoiced = await invoiceGenerator.IsInvoicedAsync(subscription.WorkspaceId, periodStart);
            }

            await uow.CompleteAsync();
            return invoiced;
        }
        catch (Exception ex)
        {
            Logger.LogWarning(ex,
                "Could not check whether workspace {WorkspaceId} is already invoiced for {Period:yyyy-MM}.",
                subscription.WorkspaceId, periodStart);
            return false;
        }
    }
}
