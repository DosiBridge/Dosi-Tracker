namespace Dosi.Tracker.SaaS;

/// <summary>What one run of <see cref="TrialExpiryWorker.CheckSubscriptionsAsync"/> did.</summary>
public class SubscriptionSweepResult
{
    /// <summary>The sweep did not run because another one held the sweep lock.</summary>
    public bool Skipped { get; init; }

    /// <summary>The sweep did not run because billing is off on this deployment (<see cref="Billing.BillingOptions"/>).</summary>
    public bool BillingDisabled { get; init; }

    public int TrialRemindersSent { get; set; }

    /// <summary>Trials that ended and were marked past_due.</summary>
    public int TrialsEnded { get; set; }

    /// <summary>Active workspaces marked past_due because an invoice went unpaid past its due date.</summary>
    public int MarkedPastDue { get; set; }

    /// <summary>Subscriptions that could not be reviewed; the next sweep tries again.</summary>
    public int Failed { get; set; }

    public void Add(SubscriptionReviewOutcome outcome)
    {
        switch (outcome)
        {
            case SubscriptionReviewOutcome.TrialReminderSent:
                TrialRemindersSent++;
                break;
            case SubscriptionReviewOutcome.TrialEnded:
                TrialsEnded++;
                break;
            case SubscriptionReviewOutcome.MarkedPastDue:
                MarkedPastDue++;
                break;
        }
    }
}
