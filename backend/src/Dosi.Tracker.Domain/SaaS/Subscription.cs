using System;
using Volo.Abp.Data;
using Volo.Abp.Domain.Entities.Auditing;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// A workspace's plan and billing state. A paid plan starts <c>trialing</c>; when the trial ends the workspace is
/// <c>past_due</c> until the host activates it. An <c>active</c> paid workspace is invoiced monthly, and an invoice
/// left unpaid past its due date makes it <c>past_due</c> again until that invoice is paid. Free plans are always
/// <c>active</c>. <c>canceled</c> (its workspace was deleted), <c>suspended</c> (by the host) and <c>expired</c> end
/// access. What each status allows is decided by <see cref="SubscriptionAccessChecker"/>.
/// </summary>
public class Subscription : FullAuditedAggregateRoot<Guid>, IWorkspaceScoped
{
    public const string TrialingStatus = "trialing";
    public const string ActiveStatus = "active";
    public const string PastDueStatus = "past_due";
    public const string CanceledStatus = "canceled";
    public const string ExpiredStatus = "expired";
    public const string SuspendedStatus = "suspended";

    /// <summary>Days a past_due workspace stays usable after its oldest unpaid invoice fell due.</summary>
    public const int PastDueGraceDays = 7;

    /// <summary>How many days before a trial ends the workspace owners are reminded of it.</summary>
    public const int TrialReminderDays = 3;

    /// <summary>Extra property set once the owners have been reminded that the trial is ending (sent once).</summary>
    public const string TrialReminderSentProperty = "TrialReminderSent";

    public Guid WorkspaceId { get; protected set; }
    public Guid PlanId { get; set; }
    public string Status { get; set; } // one of the *Status constants
    public DateTime? TrialEndsAt { get; set; }

    protected Subscription() { }

    public Subscription(Guid id, Guid workspaceId, Guid planId)
        : base(id)
    {
        WorkspaceId = workspaceId;
        PlanId = planId;
        Status = TrialingStatus;
    }

    /// <summary>A trial whose end has passed, whether or not the hourly sweep has marked it past_due yet.
    /// A trial without an end date never ends.</summary>
    public bool IsTrialOver(DateTime now)
    {
        return Status == TrialingStatus && TrialEndsAt.HasValue && TrialEndsAt.Value < now;
    }

    public void StartTrial(DateTime endsAt)
    {
        Status = TrialingStatus;
        TrialEndsAt = endsAt;
    }

    /// <summary>Paying (or on a free plan) and usable; a running trial ends here.</summary>
    public void Activate()
    {
        Status = ActiveStatus;
        TrialEndsAt = null;
    }

    public void MarkPastDue()
    {
        Status = PastDueStatus;
    }

    public void Suspend()
    {
        Status = SuspendedStatus;
    }

    public void Cancel()
    {
        Status = CanceledStatus;
    }

    /// <summary>Moves the workspace to <paramref name="plan"/>. A free plan has nothing to pay or trial, so the
    /// subscription becomes active; on a paid plan its status (trial, active, past_due, …) is kept.</summary>
    public void ChangePlan(Plan plan)
    {
        PlanId = plan.Id;
        if (plan.IsFree())
        {
            Activate();
        }
    }

    public bool IsTrialReminderSent()
    {
        return this.GetProperty<bool>(TrialReminderSentProperty);
    }

    public void MarkTrialReminderSent()
    {
        this.SetProperty(TrialReminderSentProperty, true);
    }
}
