using System;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Devices;

/// <summary>A workspace owner or manager revoking (or restoring) a device of a member.</summary>
public class RevokeMemberDeviceInput
{
    [Required]
    public Guid UserId { get; set; }

    [Required]
    public Guid DeviceId { get; set; }

    [MaxLength(DeviceConsts.MaxReasonLength)]
    public string? Reason { get; set; }
}

/// <summary>A workspace owner or manager lifting the revocation of a member's device.</summary>
public class RestoreDeviceInput
{
    [Required]
    public Guid UserId { get; set; }

    [Required]
    public Guid DeviceId { get; set; }
}
