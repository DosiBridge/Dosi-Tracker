using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Text.Json.Serialization;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.Activities;

/// <summary>
/// One tracked block uploaded by a desktop agent (shared contract §1). The window lists, the timeline and the
/// captures are kept out of the audit log: they are large, and they may hold data the project does not allow,
/// which the ingest service drops before storing the block.
/// </summary>
public class CreateActivityDto
{
    /// <summary>Upper bound on the ActiveWindows / RunningPrograms lists of a single block.</summary>
    public const int MaxWindowEntries = 200;

    /// <summary>Upper bound on the per-minute <see cref="Timeline"/> of a single block (blocks are at most 120 min).</summary>
    public const int MaxTimelineEntries = 120;

    [Required]
    public Guid ProjectId { get; set; }

    [Required]
    public Guid ClientActivityId { get; set; }

    [Required]
    public DateTime StartedAt { get; set; }

    [Required]
    public DateTime EndedAt { get; set; }

    /// <summary>Activity % of the block; null = not measured (input capture unavailable, denied or not allowed).
    /// Older agents always send a number.</summary>
    [Range(0, 100)]
    public int? Productivity { get; set; }

    /// <summary>Mouse clicks during the block; null (or absent) = not measured — input counting is off, not consented
    /// or unsupported in the session (e.g. Wayland). Send 0 only for a measured block without clicks. Older agents
    /// always send a number.</summary>
    [Range(0, int.MaxValue)]
    public int? MouseClicks { get; set; }

    /// <summary>Keystrokes during the block (a count only); null (or absent) = not measured, as for
    /// <see cref="MouseClicks"/>.</summary>
    [Range(0, int.MaxValue)]
    public int? KeyboardHits { get; set; }

    [MaxLength(2048)]
    public string? Description { get; set; }

    // Accept structured data from clients
    /// <summary>Per-(app, window title) usage during the whole block, most-used first. Older agents send a
    /// single entry (the window focused at capture time) without <see cref="WindowInfoDto.Seconds"/>.</summary>
    [MaxLength(MaxWindowEntries)]
    [DisableAuditing]
    public List<WindowInfoDto> ActiveWindows { get; set; } = new();

    /// <summary>Applications running during the block. Only their names are kept: window titles of background
    /// windows are never stored (agents send them empty; the server blanks them for older agents).</summary>
    [MaxLength(MaxWindowEntries)]
    [DisableAuditing]
    public List<WindowInfoDto> RunningPrograms { get; set; } = new();

    /// <summary>Optional per-minute breakdown of the block (older agents omit it).</summary>
    [MaxLength(MaxTimelineEntries)]
    [DisableAuditing]
    public List<ActivityMinuteDto> Timeline { get; set; } = new();

    // Accept base64 images from desktop agents. An invalid, oversized or disallowed capture is dropped (logged)
    // and the block is still stored. Limits (decoded): screen 8 MB, thumbnail 512 KB, webcam 2 MB.
    /// <summary>The screen capture as JPEG (preferred) or PNG; the stored format is detected from its bytes.</summary>
    [DisableAuditing]
    public string? ScreenshotPngBase64 { get; set; }

    /// <summary>Optional small JPEG rendition (at most 480px wide) of the same screenshot, used for list thumbnails.</summary>
    [DisableAuditing]
    public string? ScreenshotThumbJpgBase64 { get; set; }

    /// <summary>Optional webcam frame (JPEG).</summary>
    [DisableAuditing]
    public string? WebcamJpgBase64 { get; set; }

    // Optional block context (newer agents). Never a reason to reject a block: the server normalizes these
    // values (trims/truncates strings, stores an unknown or out-of-range value as null), so the limits below
    // (see ActivityConsts) are documented rather than enforced by validation.

    /// <summary>"windows" or "macos" (at most <see cref="ActivityConsts.MaxPlatformLength"/> characters).</summary>
    public string? Platform { get; set; }

    /// <summary>e.g. "Windows 11 Pro 23H2 (10.0.22631)" (at most <see cref="ActivityConsts.MaxOsVersionLength"/> characters).</summary>
    public string? OsVersion { get; set; }

    /// <summary>e.g. "0.2.0" (at most <see cref="ActivityConsts.MaxAgentVersionLength"/> characters).</summary>
    public string? AgentVersion { get; set; }

    /// <summary>Random id generated once per agent install.</summary>
    public Guid? DeviceId { get; set; }

    /// <summary>The computer name (at most <see cref="ActivityConsts.MaxDeviceNameLength"/> characters).</summary>
    public string? DeviceName { get; set; }

    /// <summary>IANA time zone id of the device, e.g. "Asia/Dhaka" (at most <see cref="ActivityConsts.MaxTimeZoneLength"/> characters).</summary>
    public string? TimeZone { get; set; }

    /// <summary>The device's UTC offset at <see cref="StartedAt"/>, in minutes
    /// (<see cref="ActivityConsts.MinUtcOffsetMinutes"/>..<see cref="ActivityConsts.MaxUtcOffsetMinutes"/>).</summary>
    public int? UtcOffsetMinutes { get; set; }

    /// <summary>Seconds of the block without keyboard/mouse input (≥ 0): the length of the timeline buckets whose
    /// Active flag is false. A block kept after an idle prompt reports its full length. The server clamps it to
    /// the block length, and derives it from the <see cref="Timeline"/> the same way when it is absent.</summary>
    public int? IdleSeconds { get; set; }

    /// <summary>Why the block ended: one of <see cref="ActivityConsts.EndReasons"/>.</summary>
    public string? EndReason { get; set; }

    /// <summary>The device's UTC clock when the upload was sent; the server stores the clock skew
    /// (server time − <see cref="SentAt"/>, clamped to ±10 days) with the block.</summary>
    public DateTime? SentAt { get; set; }

    /// <summary>When the screen capture was taken (agents capture at a random moment inside the block). Used as the
    /// capture time of the screen and its thumbnail when it lies within the block (±60 s); otherwise the block's end.</summary>
    public DateTime? ScreenshotCapturedAt { get; set; }

    /// <summary>True when the agent blurred the screen capture and its thumbnail before uploading them. When the
    /// project requires blurred screenshots and this is not true, the server drops both (the block is kept).</summary>
    public bool? ScreenshotBlurred { get; set; }

    /// <summary>Software-injected keyboard/mouse events seen during the block (≥ 0); they are excluded from the
    /// counts and from <see cref="Productivity"/>.</summary>
    public int? InjectedInputEvents { get; set; }
}

/// <summary>
/// A window/application seen during a block. Also the shape persisted in <c>Activity.ActiveWindowsJson</c> /
/// <c>RunningProgramsJson</c>; the optional usage fields are omitted from the stored JSON when absent so
/// legacy entries keep their original <c>{"AppName":...,"WindowTitle":...}</c> shape.
/// </summary>
public class WindowInfoDto
{
    public string? AppName { get; set; }
    public string? WindowTitle { get; set; }

    /// <summary>Foreground seconds attributed to this window during the block (null from older agents).</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? Seconds { get; set; }

    /// <summary>Keystrokes attributed to this window during the block.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? KeyboardHits { get; set; }

    /// <summary>Mouse clicks attributed to this window during the block.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? MouseClicks { get; set; }
}

/// <summary>
/// One minute of a block's timeline. Also the shape persisted in <c>Activity.TimelineJson</c>
/// (<see cref="AppName"/> and unmeasured counts are omitted from the stored JSON when absent).
/// </summary>
public class ActivityMinuteDto
{
    /// <summary>0-based minute bucket index, counted from the block's StartedAt.</summary>
    public int Minute { get; set; }

    /// <summary>Keystrokes of this minute; null = not measured (omitted from the stored JSON then).</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? KeyboardHits { get; set; }

    /// <summary>Mouse clicks of this minute; null = not measured (omitted from the stored JSON then).</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? MouseClicks { get; set; }

    /// <summary>Whether the member was active (not idle) during this minute.</summary>
    public bool Active { get; set; }

    /// <summary>The application mostly in the foreground during this minute, if known.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? AppName { get; set; }

    /// <summary>Seconds of this bucket with any input (keyboard, click, mouse move or wheel), 0..90 (the last
    /// bucket can absorb a remainder); null from older agents, and omitted from the stored JSON then.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? ActiveSeconds { get; set; }
}
