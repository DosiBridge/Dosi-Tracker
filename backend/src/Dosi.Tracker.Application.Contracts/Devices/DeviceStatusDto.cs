using System;

namespace Dosi.Tracker.Devices;

public class DeviceStatusDto
{
    public Guid DeviceId { get; set; }
    public bool IsRevoked { get; set; }
    public DateTime? RevokedAt { get; set; }
    public string? Reason { get; set; }
}
