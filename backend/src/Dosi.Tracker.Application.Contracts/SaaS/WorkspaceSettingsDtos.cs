using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.SaaS;

/// <summary>Workspace settings (<c>GET/PUT /api/app/workspace/settings</c>): the workspace's default reporting time
/// zone and what new projects start with. Every member can read them; only the workspace owner changes them.</summary>
public class WorkspaceSettingsDto
{
    /// <summary>IANA time zone id ("Asia/Dhaka"); null when not set, so reports fall back to the viewer's own zone.
    /// The effective reporting zone is the user's preference, then this, then the browser's.
    /// A Windows id is accepted and stored as its IANA equivalent; an unknown id is a validation error.</summary>
    [MaxLength(ActivityConsts.MaxTimeZoneLength)]
    public string? DefaultTimeZone { get; set; }

    [Required]
    public NewProjectDefaultsDto NewProjectDefaults { get; set; } = new();
}

/// <summary>What a new project starts with; the same rules as the project's own settings.</summary>
public class NewProjectDefaultsDto
{
    [Range(5, 60)]
    public int IntervalMinutes { get; set; } = 10;

    public bool AllowScreenshot { get; set; } = true;
    public bool AllowWebcam { get; set; } = false;
    public bool AllowKeyboard { get; set; } = true;
    public bool AllowMouse { get; set; } = true;
    public bool AllowActiveWindow { get; set; } = true;
    public bool AllowRunningPrograms { get; set; } = true;
    public bool BlurScreenshots { get; set; } = false;

    /// <summary>0 turns idle detection off.</summary>
    [Range(0, 60)]
    public int IdleThresholdMinutes { get; set; } = 5;
}

/// <summary>The signed-in user's own preferences (<c>GET/PUT /api/app/workspace/my-preferences</c>).</summary>
public class MyPreferencesDto
{
    /// <summary>IANA time zone id the user reports in; null when not set (the workspace default applies).
    /// A Windows id is accepted and stored as its IANA equivalent; an unknown id is a validation error.</summary>
    [MaxLength(ActivityConsts.MaxTimeZoneLength)]
    public string? TimeZone { get; set; }
}
