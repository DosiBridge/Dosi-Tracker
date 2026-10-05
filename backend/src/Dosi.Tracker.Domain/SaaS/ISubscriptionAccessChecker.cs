using System.Threading.Tasks;

namespace Dosi.Tracker.SaaS;

/// <summary>Decides whether the current workspace's subscription allows it to use the product
/// (for example to record tracked time). Callers reject with <c>Tracker:SubscriptionInactive</c> otherwise.</summary>
public interface ISubscriptionAccessChecker
{
    /// <summary>True for the host, for a workspace without a subscription, for an active subscription, for a trial
    /// that has not ended, and for a past_due subscription during the grace period of
    /// <see cref="Subscription.PastDueGraceDays"/> days after its oldest unpaid invoice fell due (not counting invoices
    /// the host excused by reactivating the workspace). False for an ended trial, a past_due workspace past its grace
    /// period or with nothing unpaid (its trial ended), and for canceled, expired, suspended or unknown statuses.</summary>
    Task<bool> IsUsableAsync();
}
