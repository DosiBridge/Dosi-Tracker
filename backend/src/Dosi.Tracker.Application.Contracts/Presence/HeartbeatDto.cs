using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using Dosi.Tracker.Activities;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.Presence;

/// <summary>
/// The live state of one desktop agent install, sent every 60 s while it runs and signed in and immediately on
/// every status change (shared contract §3). Only <see cref="DeviceId"/> and <see cref="Status"/> are required;
/// the optional fields are normalized like the block context of an activity upload (an invalid value is stored
/// as null) and never make a heartbeat fail.
/// </summary>
public class HeartbeatDto : IValidatableObject
{
    /// <summary>Random id generated once per agent install.</summary>
    [Required]
    public Guid DeviceId { get; set; }

    /// <summary>The computer name (at most <see cref="ActivityConsts.MaxDeviceNameLength"/> characters).</summary>
    public string? DeviceName { get; set; }

    /// <summary>"windows" or "macos".</summary>
    public string? Platform { get; set; }

    public string? OsVersion { get; set; }

    public string? AgentVersion { get; set; }

    /// <summary>One of <see cref="AgentPresenceConsts.Statuses"/>; anything else is a validation error.</summary>
    [Required]
    [StringLength(AgentPresenceConsts.MaxStatusLength)]
    public string Status { get; set; } = string.Empty;

    /// <summary>The project being tracked (or last chosen), if any.</summary>
    public Guid? ProjectId { get; set; }

    /// <summary>When the agent entered <see cref="Status"/>, by the device's clock.</summary>
    public DateTime? StatusSince { get; set; }

    /// <summary>The foreground application. Kept only when the project allows the active window.</summary>
    [DisableAuditing]
    public string? CurrentApp { get; set; }

    /// <summary>IANA time zone id of the device, e.g. "Asia/Dhaka".</summary>
    public string? TimeZone { get; set; }

    /// <summary>The device's current UTC offset in minutes.</summary>
    public int? UtcOffsetMinutes { get; set; }

    /// <summary>The device's UTC clock when the heartbeat was sent; used to bring <see cref="StatusSince"/> onto
    /// the server's clock.</summary>
    public DateTime? SentAt { get; set; }

    public IEnumerable<ValidationResult> Validate(ValidationContext validationContext)
    {
        if (DeviceId == Guid.Empty)
        {
            yield return new ValidationResult(
                "DeviceId is required: the random id the agent generated for this install.",
                new[] { nameof(DeviceId) });
        }

        if (!string.IsNullOrWhiteSpace(Status) && FindStatus(Status) == null)
        {
            yield return new ValidationResult(
                $"Status must be one of: {string.Join(", ", AgentPresenceConsts.Statuses)}.",
                new[] { nameof(Status) });
        }
    }

    /// <summary>The known status <paramref name="status"/> names (case and surrounding spaces ignored), or null.</summary>
    public static string? FindStatus(string? status)
    {
        var value = status?.Trim();
        return string.IsNullOrEmpty(value)
            ? null
            : AgentPresenceConsts.Statuses.FirstOrDefault(s => string.Equals(s, value, StringComparison.OrdinalIgnoreCase));
    }
}

/// <summary>The answer to a heartbeat.</summary>
public class HeartbeatResultDto
{
    /// <summary>The server's UTC clock; agents use it to detect a wrong device clock.</summary>
    public DateTime ServerTime { get; set; }
}
