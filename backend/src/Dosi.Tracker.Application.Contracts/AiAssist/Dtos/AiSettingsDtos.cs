using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.AiAssist;

/// <summary>What AI assist sends and to whom, as owners must read it before they turn it on. Server-localized.</summary>
public class AiDisclosureDto
{
    /// <summary>The version an owner acknowledges (<see cref="UpdateAiWorkspaceSettingsDto.AcknowledgedDisclosureVersion"/>).</summary>
    public string Version { get; set; } = string.Empty;

    /// <summary>Who processes the data.</summary>
    public string ProviderDisplayName { get; set; } = string.Empty;

    /// <summary>Where it is processed, when the operator says.</summary>
    public string? ProcessingRegion { get; set; }

    /// <summary>What the provider keeps, when the operator says.</summary>
    public string? RetentionNote { get; set; }

    public List<string> AlwaysSent { get; set; } = new();
    public List<string> SentWhenAllowed { get; set; } = new();
    public List<string> NeverSent { get; set; } = new();
    public List<string> WhoCanUse { get; set; } = new();
}

/// <summary>A workspace's AI assist settings (<c>GET /api/app/ai-assist/settings</c>, owners and managers).</summary>
public class AiWorkspaceSettingsDto
{
    public bool IsEnabled { get; set; }
    public bool AllowManagers { get; set; }
    public bool AllowMemberSelfService { get; set; }
    public bool AllowWindowTitles { get; set; }
    public bool AllowNotes { get; set; }

    /// <summary>The disclosure version the owner last consented to; null if they never did.</summary>
    public string? ConsentVersion { get; set; }

    /// <summary>When AI assist was last turned on or re-consented (UTC).</summary>
    public DateTime? EnabledAt { get; set; }

    /// <summary>Who did.</summary>
    public string? EnabledByName { get; set; }

    public AiDisclosureDto Disclosure { get; set; } = new();
}

/// <summary>
/// An owner's change (<c>PUT /api/app/ai-assist/settings</c>). Enabling, re-consenting (the disclosure changed since the
/// last consent), or turning on window titles or notes requires <see cref="AcknowledgedDisclosureVersion"/> to be the
/// current disclosure version; otherwise <c>Tracker:AiDisclosureNotAccepted</c> (400, <c>data.version</c>).
/// </summary>
public class UpdateAiWorkspaceSettingsDto
{
    public bool IsEnabled { get; set; }
    public bool AllowManagers { get; set; } = true;
    public bool AllowMemberSelfService { get; set; } = true;
    public bool AllowWindowTitles { get; set; }
    public bool AllowNotes { get; set; }

    [StringLength(16)]
    public string? AcknowledgedDisclosureVersion { get; set; }
}
