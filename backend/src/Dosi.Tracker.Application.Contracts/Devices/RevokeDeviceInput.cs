using System;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Devices;

public class RevokeDeviceInput
{
    [Required]
    public Guid DeviceId { get; set; }

    [MaxLength(DeviceConsts.MaxReasonLength)]
    public string? Reason { get; set; }
}
