using System;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Microsoft.Extensions.Logging;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker.SaaS;

/// <summary>What one sweep step (<see cref="SubscriptionManager.ReviewAsync"/>) did to a subscription.</summary>
public enum SubscriptionReviewOutcome
{
    None,
    TrialReminderSent,
    TrialEnded,
    MarkedPastDue
}

/// <summary>
/// The subscription lifecycle of the current workspace: how a new workspace starts, the host's plan and status
/// changes, recording a payment, and the hourly review that ends trials and acts on overdue invoices.
/// </summary>
public class SubscriptionManager : DomainService
{
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Invoice, Guid> _invoiceRepository;
    private readonly SeatCounter _seatCounter;
    private readonly SubscriptionAccessChecker _accessChecker;
    private readonly WorkspaceOwnerNotifier _ownerNotifier;

    public SubscriptionManager(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Invoice, Guid> invoiceRepository,
        SeatCounter seatCounter,
        SubscriptionAccessChecker accessChecker,
        WorkspaceOwnerNotifier ownerNotifier)
    {
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceRepository = invoiceRepository;
        _seatCounter = seatCounter;
        _accessChecker = accessChecker;
        _ownerNotifier = ownerNotifier;
    }

    /// <summary>The subscription a new workspace starts with (not yet saved): a paid plan with trial days starts a
    /// trial; a free plan, and a paid plan without a trial (which only the host can assign), is active.</summary>
    public virtual Subscription CreateForNewWorkspace(Guid? tenantId, Plan plan)
    {
        var subscription = new Subscription(GuidGenerator.Create(), tenantId, plan.Id);
        if (!plan.IsFree() && plan.TrialDays > 0)
        {
            subscription.StartTrial(Clock.Now.AddDays(plan.TrialDays));
        }
        else
        {
            subscription.Activate();
        }

        return subscription;
    }

    public virtual Task<Subscription?> FindCurrentAsync()
    {
        return _subscriptionRepository.FirstOrDefaultAsync(s => s.TenantId == CurrentTenant.Id);
    }

    /// <summary>Host: moves the current workspace to <paramref name="plan"/> (see <see cref="Subscription.ChangePlan"/>);
    /// a workspace without a subscription gets an active one. Refused with <c>Tracker:SeatLimitReached</c> when the
    /// workspace occupies more seats than the plan allows.</summary>
    public virtual async Task<Subscription> ChangePlanAsync(Plan plan)
    {
        await _seatCounter.CheckFitsPlanAsync(plan);

        var subscription = await FindCurrentAsync();
        if (subscription == null)
        {
            subscription = new Subscription(GuidGenerator.Create(), CurrentTenant.Id, plan.Id);
            subscription.Activate();
            await _subscriptionRepository.InsertAsync(subscription, autoSave: true);
        }
        else
        {
            subscription.ChangePlan(plan);
            await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);
        }

        Logger.LogInformation("Workspace {TenantId} moved to the '{Plan}' plan ({Status}).", CurrentTenant.Id, plan.Name, subscription.Status);
        return subscription;
    }

    /// <summary>Host: marks the current workspace's subscription active (a trial or a past_due workspace the host
    /// has settled with). The invoices overdue at that moment stay owed (still unpaid) but are excused
    /// (<see cref="Invoice.ExcuseOverdue"/>): they no longer hold the workspace past due, so the next invoice that goes
    /// unpaid gets its full grace period, and paying that one reactivates the workspace.</summary>
    public virtual async Task<Subscription> ActivateAsync()
    {
        var subscription = await GetCurrentAsync();
        subscription.Activate();
        await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);

        var now = Clock.Now;
        var excused = (await _invoiceRepository.GetListAsync(i => i.Status != Invoice.PaidStatus && i.DueDate < now))
            .Where(i => !i.IsOverdueExcused())
            .ToList();
        foreach (var invoice in excused)
        {
            invoice.ExcuseOverdue();
        }

        if (excused.Count > 0)
        {
            await _invoiceRepository.UpdateManyAsync(excused, autoSave: true);
        }

        Logger.LogInformation("Subscription of workspace {TenantId} activated ({Count} overdue invoice(s) excused).",
            CurrentTenant.Id, excused.Count);
        return subscription;
    }

    /// <summary>Host: suspends the current workspace; it is not usable until activated again.</summary>
    public virtual async Task<Subscription> SuspendAsync()
    {
        var subscription = await GetCurrentAsync();
        subscription.Suspend();
        await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);

        Logger.LogInformation("Subscription of workspace {TenantId} suspended.", CurrentTenant.Id);
        return subscription;
    }

    /// <summary>
    /// Records the payment of an invoice of the current workspace. When it was overdue and was the workspace's last
    /// overdue invoice (invoices the host excused do not count), a past_due workspace becomes active again. Paying an
    /// already paid invoice changes nothing. Returns true when the subscription was reactivated.
    /// </summary>
    public virtual async Task<bool> MarkInvoicePaidAsync(Invoice invoice)
    {
        if (invoice.IsPaid())
        {
            return false;
        }

        var now = Clock.Now;
        var wasOverdue = invoice.IsOverdue(now);
        invoice.MarkPaid();
        await _invoiceRepository.UpdateAsync(invoice, autoSave: true);

        if (!wasOverdue)
        {
            return false;
        }

        var subscription = await FindCurrentAsync();
        if (subscription == null || subscription.Status != Subscription.PastDueStatus)
        {
            return false;
        }

        var paidInvoiceId = invoice.Id;
        var anotherOverdue = (await _invoiceRepository.GetListAsync(
                i => i.Id != paidInvoiceId && i.Status != Invoice.PaidStatus && i.DueDate < now))
            .Any(i => i.HoldsWorkspacePastDue(now));
        if (anotherOverdue)
        {
            return false;
        }

        subscription.Activate();
        await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);

        Logger.LogInformation("Workspace {TenantId} paid its overdue invoices and is active again.", CurrentTenant.Id);
        return true;
    }

    /// <summary>
    /// One step of the hourly sweep, for a subscription of the current workspace: reminds the owners
    /// <see cref="Subscription.TrialReminderDays"/> days before a trial ends (once), marks an ended trial past_due,
    /// and marks an active workspace past_due when an invoice has gone unpaid past its due date (once per invoice,
    /// and never for an invoice the host excused, so a workspace the host has activated again is not suspended anew
    /// by the same invoice). The owners are notified of each change. Changes are saved with the caller's unit of work.
    /// </summary>
    public virtual async Task<SubscriptionReviewOutcome> ReviewAsync(Subscription subscription)
    {
        var now = Clock.Now;
        return subscription.Status switch
        {
            Subscription.TrialingStatus => await ReviewTrialAsync(subscription, now),
            Subscription.ActiveStatus => await ReviewInvoicesAsync(subscription, now),
            _ => SubscriptionReviewOutcome.None
        };
    }

    private async Task<SubscriptionReviewOutcome> ReviewTrialAsync(Subscription subscription, DateTime now)
    {
        if (!subscription.TrialEndsAt.HasValue)
        {
            return SubscriptionReviewOutcome.None;
        }

        if (subscription.IsTrialOver(now))
        {
            subscription.MarkPastDue();
            await _subscriptionRepository.UpdateAsync(subscription);
            await _ownerNotifier.NotifyAsync(
                $"Your {await GetPlanNameAsync(subscription)} trial has ended. Time tracking is paused until the " +
                "subscription is activated; contact support to continue.");

            Logger.LogInformation(
                "Subscription {SubscriptionId} (tenant {TenantId}) trial ended; marked past_due.",
                subscription.Id, subscription.TenantId);
            return SubscriptionReviewOutcome.TrialEnded;
        }

        if (subscription.TrialEndsAt.Value <= now.AddDays(Subscription.TrialReminderDays) &&
            !subscription.IsTrialReminderSent())
        {
            subscription.MarkTrialReminderSent();
            await _subscriptionRepository.UpdateAsync(subscription);
            await _ownerNotifier.NotifyAsync(
                $"Your {await GetPlanNameAsync(subscription)} trial ends on {FormatDate(subscription.TrialEndsAt.Value)} " +
                "(UTC). Contact support before then to keep tracking time.");
            return SubscriptionReviewOutcome.TrialReminderSent;
        }

        return SubscriptionReviewOutcome.None;
    }

    private async Task<SubscriptionReviewOutcome> ReviewInvoicesAsync(Subscription subscription, DateTime now)
    {
        var newlyOverdue = (await _invoiceRepository.GetListAsync(i => i.Status != Invoice.PaidStatus && i.DueDate < now))
            .Where(i => i.HoldsWorkspacePastDue(now) && !i.IsOverdueNoticeSent())
            .OrderBy(i => i.DueDate)
            .ToList();
        if (newlyOverdue.Count == 0)
        {
            return SubscriptionReviewOutcome.None;
        }

        subscription.MarkPastDue();
        await _subscriptionRepository.UpdateAsync(subscription);
        foreach (var invoice in newlyOverdue)
        {
            invoice.MarkOverdueNoticeSent();
            await _invoiceRepository.UpdateAsync(invoice);
        }

        var oldest = newlyOverdue[0];
        var graceEndsAt = await _accessChecker.GetPastDueGraceEndAsync() ?? oldest.DueDate.AddDays(Subscription.PastDueGraceDays);
        await _ownerNotifier.NotifyAsync(
            $"The invoice of {oldest.Amount.ToString("0.00", CultureInfo.InvariantCulture)} due on " +
            $"{FormatDate(oldest.DueDate)} has not been paid. Time tracking stays available until " +
            $"{FormatDate(graceEndsAt)} (UTC); settle the invoice to keep it running.");

        Logger.LogInformation(
            "Subscription {SubscriptionId} (tenant {TenantId}) has {Count} overdue invoice(s); marked past_due.",
            subscription.Id, subscription.TenantId, newlyOverdue.Count);
        return SubscriptionReviewOutcome.MarkedPastDue;
    }

    private async Task<Subscription> GetCurrentAsync()
    {
        return await FindCurrentAsync() ?? throw new EntityNotFoundException(typeof(Subscription));
    }

    private async Task<string> GetPlanNameAsync(Subscription subscription)
    {
        var plan = await _planRepository.FindAsync(subscription.PlanId);
        return plan?.Name ?? "current";
    }

    private static string FormatDate(DateTime value)
    {
        return value.ToString("MMMM d, yyyy", CultureInfo.InvariantCulture);
    }
}
