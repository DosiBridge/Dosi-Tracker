using System;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The consent history of AI assist: one row each time an owner turned it on or off, re-consented to a changed
/// disclosure, or changed what it may use. Independent of audit-log retention; kept as long as the workspace.
/// </summary>
public class AiConsentEvent : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public const int MaxActionLength = 24;
    public const int MaxDisclosureVersionLength = 16;
    public const int MaxFlagsLength = 256;

    public const string EnabledAction = "enabled";
    public const string DisabledAction = "disabled";
    public const string OptionsChangedAction = "options-changed";
    public const string ReconsentedAction = "reconsented";

    public Guid WorkspaceId { get; private set; }

    /// <summary>The owner who made the change.</summary>
    public Guid UserId { get; private set; }

    /// <summary><see cref="EnabledAction"/>, <see cref="DisabledAction"/>, <see cref="OptionsChangedAction"/> or
    /// <see cref="ReconsentedAction"/>.</summary>
    public string Action { get; private set; } = null!;

    /// <summary>The disclosure version current at the change ("{code}.{operator}").</summary>
    public string DisclosureVersion { get; private set; } = null!;

    /// <summary>The options after the change (<see cref="AiWorkspacePolicy.Flags"/>).</summary>
    public string Flags { get; private set; } = null!;

    protected AiConsentEvent()
    {
    }

    public AiConsentEvent(Guid id, Guid workspaceId, Guid userId, string action, string disclosureVersion, string flags)
    {
        Id = id;
        WorkspaceId = workspaceId;
        UserId = userId;
        Action = Check.NotNullOrWhiteSpace(action, nameof(action), MaxActionLength);
        DisclosureVersion = Check.NotNullOrWhiteSpace(disclosureVersion, nameof(disclosureVersion), MaxDisclosureVersionLength);
        Flags = Check.NotNull(flags, nameof(flags), MaxFlagsLength);
    }
}
