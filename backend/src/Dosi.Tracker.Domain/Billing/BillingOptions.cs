namespace Dosi.Tracker.Billing;

/// <summary>
/// The deployment-level billing switch, <c>Billing:Enabled</c> (environment variable <c>Billing__Enabled</c>).
/// <para>Off — the default, meant for self-hosted deployments — there is no paywall: every workspace is usable whatever
/// its subscription says, plans set no seat limit, a new workspace starts active instead of on a trial, and neither
/// trials nor invoices are swept. Owners cannot change or cancel a plan (<c>Tracker:BillingDisabled</c>).</para>
/// <para>On — the hosted SaaS — plans, trials, seat limits, monthly invoices and the subscription sweep apply, and owners
/// can move to a free plan whose seats fit, or cancel.</para>
/// </summary>
public class BillingOptions
{
    public const string SectionName = "Billing";

    public bool Enabled { get; set; }
}
