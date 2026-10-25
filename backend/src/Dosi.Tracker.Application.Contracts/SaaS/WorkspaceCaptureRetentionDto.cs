using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// Workspace capture retention policy (GET/PUT /api/app/workspaces/{id}/capture-retention).
/// Safe default-off: IsEnabled defaults to false.
/// Positive bounded retention days with documented supported range (1 to 730 days).
/// Authenticated workspace members can read; only active workspace owners can update.
/// </summary>
public class WorkspaceCaptureRetentionDto
{
    public const int MinRetentionDays = 1;
    public const int MaxRetentionDays = 730;
    public const int DefaultDays = 30;

    /// <summary>
    /// Explicit workspace opt-in for automated periodic capture retention sweeps.
    /// Safe default-off (false). When false or unset, background sweeps never purge captures for this workspace.
    /// </summary>
    public bool IsEnabled { get; set; } = false;

    /// <summary>
    /// Bounded capture retention duration in days.
    /// Supported range: 1 to 730 days (~2 years). Default: 30 days.
    /// Captures older than Clock.Now - RetentionDays are eligible for automated cleanup when IsEnabled is true.
    /// </summary>
    [Range(MinRetentionDays, MaxRetentionDays, ErrorMessage = "Retention days must be between 1 and 730.")]
    public int RetentionDays { get; set; } = DefaultDays;

    /// <summary>
    /// Read-only (ignored on PUT): whether this server runs the retention sweeps at all. False when the operator turned
    /// them off (CaptureRetention:IsEnabled = false): the policy is then stored but nothing is deleted.
    /// </summary>
    public bool IsEnforced { get; set; }
}

/// <summary>
/// Workspace activity retention policy (GET/PUT /api/app/workspaces/{id}/activity-retention): how long tracked time
/// blocks are kept. Off unless the owner turns it on. Blocks that still have captures are kept until capture retention
/// removes those, and a block inside the agents' upload window is never deleted. Members read it; owners change it.
/// </summary>
public class WorkspaceActivityRetentionDto
{
    public bool IsEnabled { get; set; }

    /// <summary>Blocks that started longer ago are deleted while the policy is on
    /// (<see cref="ActivityRetentionLimits.MinDays"/>..<see cref="ActivityRetentionLimits.MaxDays"/>).</summary>
    [Range(ActivityRetentionLimits.MinDays, ActivityRetentionLimits.MaxDays)]
    public int RetentionDays { get; set; } = ActivityRetentionLimits.DefaultDays;

    /// <summary>Read-only (ignored on PUT): whether this server runs activity retention sweeps at all
    /// (ActivityRetention:IsEnabled).</summary>
    public bool IsEnforced { get; set; }
}
