using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Acceptance.Support;

/// <summary>
/// Per-scenario scratchpad shared between step classes (registered fresh in the Reqnroll scenario
/// container each scenario). Holds the entities a Given creates so a later When/Then can reference
/// them, plus the outcome of the last action (result or the exception it threw).
/// </summary>
public sealed class ScenarioState
{
    /// <summary>The fixed current-user id impersonated by the test principal accessor (the acting owner/admin).</summary>
    public static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    public Guid TenantId { get; set; }
    public string? WorkspaceName { get; set; }
    public Guid ProjectId { get; set; }

    /// <summary>User ids added/invited to the project during the scenario, in order.</summary>
    public List<Guid> MemberUserIds { get; } = new();

    /// <summary>Ids of activities submitted during the scenario, in order.</summary>
    public List<Guid> ActivityIds { get; } = new();

    /// <summary>The result of the last action (an app-service DTO, an invoice, etc.), if it succeeded.</summary>
    public object? LastResult { get; set; }

    /// <summary>The exception the last action threw, if it failed — asserted by the Then step.</summary>
    public Exception? LastException { get; set; }

    /// <summary>Billing: the subscription under test (a <c>Subscription</c>, held as object to keep this POCO dependency-free).</summary>
    public object? Subscription { get; set; }

    /// <summary>Billing: the invoice the generator produced (may be null when nothing was billable).</summary>
    public object? LastInvoice { get; set; }

    public T Result<T>() => (T)LastResult!;
}
