using Volo.Abp.Settings;

namespace Dosi.Tracker.Settings;

public class TrackerSettingDefinitionProvider : SettingDefinitionProvider
{
    public override void Define(ISettingDefinitionContext context)
    {
        //Define your own settings here. Example:
        //context.Add(new SettingDefinition(TrackerSettings.MySetting1));
    }
}
