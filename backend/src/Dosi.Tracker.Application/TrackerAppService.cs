using System;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Localization;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker;

/* Inherit your application services from this class.
 */
public abstract class TrackerAppService : ApplicationService
{
    protected TrackerAppService()
    {
        LocalizationResource = typeof(TrackerResource);
    }

    protected ICurrentWorkspace CurrentWorkspace => LazyServiceProvider.LazyGetRequiredService<ICurrentWorkspace>();

    /// <summary>The desktop agent install the caller's token was issued to (<see cref="DeviceConsts.TokenClaimType"/>),
    /// or null for a token that names none (the web dashboard).</summary>
    protected Guid? TokenDeviceId => DeviceTokenBinding.Parse(CurrentUser.FindClaim(DeviceConsts.TokenClaimType)?.Value);
}
