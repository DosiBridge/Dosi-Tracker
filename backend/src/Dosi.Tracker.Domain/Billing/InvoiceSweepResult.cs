namespace Dosi.Tracker.Billing;

/// <summary>What one run of <see cref="InvoiceGenerationWorker.GenerateInvoicesAsync"/> did.</summary>
public class InvoiceSweepResult
{
    /// <summary>The sweep did not run because another one held the sweep lock.</summary>
    public bool Skipped { get; init; }

    /// <summary>The sweep did not run because billing is off on this deployment (<see cref="BillingOptions"/>).</summary>
    public bool BillingDisabled { get; init; }

    /// <summary>Invoices created by this sweep.</summary>
    public int Generated { get; set; }

    /// <summary>Workspaces another sweep invoiced first (their insert won the unique index): already billed, not an error.</summary>
    public int AlreadyInvoiced { get; set; }

    /// <summary>Workspaces whose invoice could not be generated; the next sweep tries again.</summary>
    public int Failed { get; set; }
}
