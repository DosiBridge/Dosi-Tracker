using System.Collections.Generic;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// Whether the caller can use AI assist now (<c>GET /api/app/ai-assist/status</c>), for every member of the workspace.
/// It never fails for AI reasons: <see cref="Reason"/> names the first check that stands in the way, and
/// <see cref="Features"/> what each feature allows the caller.
/// </summary>
public class AiStatusDto
{
    public bool Available { get; set; }

    /// <summary>The first failing check (<see cref="AiStatusReasons"/>), or null when <see cref="Available"/>.</summary>
    public string? Reason { get; set; }

    /// <summary>The owner turned AI assist on (consent may still be stale: see <see cref="Reason"/>).</summary>
    public bool WorkspaceEnabled { get; set; }

    /// <summary>The caller is an active owner and can change the settings.</summary>
    public bool CanManage { get; set; }

    /// <summary>Who processes the data (<c>AI:Disclosure:ProviderDisplayName</c>); null unless this server has a provider.
    /// Never the model, the endpoint or the key.</summary>
    public string? ProviderDisplayName { get; set; }

    /// <summary>The disclosure version an owner consents to now ("1.0").</summary>
    public string DisclosureVersion { get; set; } = string.Empty;

    public bool AllowWindowTitles { get; set; }
    public bool AllowNotes { get; set; }

    /// <summary>One entry per feature (<see cref="AiFeatureIds.All"/>).</summary>
    public List<AiFeatureStatusDto> Features { get; set; } = new();

    /// <summary>The workspace's monthly allowance, for owners and managers; null for everyone else.</summary>
    public AiQuotaDto? Quota { get; set; }
}

/// <summary>Why AI assist is not available to the caller (<see cref="AiStatusDto.Reason"/>), in the order checked.</summary>
public static class AiStatusReasons
{
    /// <summary>This server has no AI provider (off, or missing a key or a model).</summary>
    public const string NotConfigured = "not-configured";

    /// <summary>The platform administrators turned AI assist off.</summary>
    public const string PlatformDisabled = "platform-disabled";

    /// <summary>The workspace's subscription does not allow use of the product.</summary>
    public const string SubscriptionInactive = "subscription-inactive";

    /// <summary>The owner has not turned AI assist on.</summary>
    public const string WorkspaceDisabled = "workspace-disabled";

    /// <summary>The disclosure changed since the owner consented; they must consent again.</summary>
    public const string ReconsentRequired = "reconsent-required";

    /// <summary>Not for the caller's role (a client; a manager the owner did not allow).</summary>
    public const string RoleNotAllowed = "role-not-allowed";

    /// <summary>The owner turned off AI on members' own data.</summary>
    public const string SelfServiceOff = "self-service-off";

    /// <summary>The workspace's monthly allowance, or the caller's daily one, is used up.</summary>
    public const string QuotaExhausted = "quota-exhausted";
}
