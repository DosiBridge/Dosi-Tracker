using Microsoft.Extensions.Localization;
using Dosi.Tracker.Localization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Ui.Branding;

namespace Dosi.Tracker;

[Dependency(ReplaceServices = true)]
public class TrackerBrandingProvider : DefaultBrandingProvider
{
    private IStringLocalizer<TrackerResource> _localizer;

    public TrackerBrandingProvider(IStringLocalizer<TrackerResource> localizer)
    {
        _localizer = localizer;
    }

    public override string AppName => _localizer["AppName"];
}
