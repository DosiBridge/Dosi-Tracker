using System.ComponentModel.DataAnnotations;

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
}
