using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Devices;

/// <summary>
/// Desktop agent installs of the current workspace. A member lists and revokes their own devices; a workspace
/// owner or manager (<c>Tracker.Devices.Manage</c>) also lists, revokes and restores other members' devices.
/// Revoking a device refuses it in the workspace and revokes its sign-in sessions (see <c>DeviceRevocationManager</c>).
/// </summary>
public interface IDeviceAppService : IApplicationService
{
    Task<DeviceRevocationDto> RevokeAsync(RevokeDeviceInput input);
    Task<ListResultDto<DeviceRevocationDto>> GetMyRevocationsAsync();
    Task<DeviceStatusDto> GetStatusAsync(Guid deviceId);

    /// <summary>The devices the caller may see in the workspace (theirs, or everyone's with <c>Tracker.Devices.Manage</c>).</summary>
    Task<ListResultDto<WorkspaceDeviceDto>> GetListAsync();

    /// <summary>Revokes a member's device (<c>Tracker.Devices.Manage</c>; only an owner may revoke an owner's device).</summary>
    Task<DeviceRevocationDto> RevokeMemberDeviceAsync(RevokeMemberDeviceInput input);

    /// <summary>Lifts a device's revocation (<c>Tracker.Devices.Manage</c>; only an owner may restore an owner's device).
    /// The install signs in again: the sessions revoked with the device stay revoked.</summary>
    Task<DeviceStatusDto> RestoreAsync(RestoreDeviceInput input);
}
