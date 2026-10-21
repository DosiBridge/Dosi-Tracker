using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Devices;

public class DeviceRevocationDto : EntityDto<Guid>
{
    public Guid WorkspaceId { get; set; }
    public Guid UserId { get; set; }
    public Guid DeviceId { get; set; }
    public DateTime RevokedAt { get; set; }
    public string? Reason { get; set; }
}
