using System;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// A workspace's AI assist settings as its owner left them (<see cref="AiSettingsManager.GetPolicyAsync"/>). AI assist
/// is on only while <see cref="IsActive"/>: enabled, and consented to the disclosure version the server shows now.
/// </summary>
public sealed record AiWorkspacePolicy(
    bool IsEnabled,
    bool AllowManagers,
    bool AllowMemberSelfService,
    bool AllowWindowTitles,
    bool AllowNotes,
    string? ConsentVersion,
    DateTime? EnabledAt,
    Guid? EnabledBy)
{
    /// <summary>A workspace whose owner never changed anything: off, managers and self-service pre-allowed, no window
    /// titles or notes.</summary>
    public static readonly AiWorkspacePolicy Default = new(
        IsEnabled: false,
        AllowManagers: true,
        AllowMemberSelfService: true,
        AllowWindowTitles: false,
        AllowNotes: false,
        ConsentVersion: null,
        EnabledAt: null,
        EnabledBy: null);

    /// <summary>On, and the owner's consent covers <paramref name="currentVersion"/>.</summary>
    public bool IsActive(string currentVersion) => IsEnabled && ConsentVersion == currentVersion;

    /// <summary>Enabled, but consented to an earlier disclosure: the owner must consent again.</summary>
    public bool NeedsReconsent(string currentVersion) => IsEnabled && ConsentVersion != currentVersion;

    /// <summary>The options as the consent history records them: "managers=1;selfService=1;windowTitles=0;notes=0".</summary>
    public string Flags =>
        $"managers={Bit(AllowManagers)};selfService={Bit(AllowMemberSelfService)};windowTitles={Bit(AllowWindowTitles)};notes={Bit(AllowNotes)}";

    private static char Bit(bool value) => value ? '1' : '0';
}

/// <summary>What an owner asks AI assist to be (<see cref="AiSettingsManager.UpdateAsync"/>).</summary>
public sealed record AiWorkspaceSettingsUpdate(
    bool IsEnabled,
    bool AllowManagers,
    bool AllowMemberSelfService,
    bool AllowWindowTitles,
    bool AllowNotes);

/// <summary>The outcome of <see cref="AiSettingsManager.UpdateAsync"/>.</summary>
/// <param name="Policy">The settings now in force.</param>
/// <param name="ConsentAction">The <see cref="AiConsentEvent"/> action that was recorded, or null when nothing
/// changed.</param>
/// <param name="NotifyMembers">Members must be told: AI assist was turned on (or re-consented), or it may now send
/// more of their data (window titles or notes).</param>
/// <param name="WindowTitlesAdded">Window titles were turned on while AI assist is on.</param>
/// <param name="NotesAdded">Task notes were turned on while AI assist is on.</param>
public sealed record AiSettingsChange(
    AiWorkspacePolicy Policy,
    string? ConsentAction,
    bool NotifyMembers,
    bool WindowTitlesAdded,
    bool NotesAdded);
