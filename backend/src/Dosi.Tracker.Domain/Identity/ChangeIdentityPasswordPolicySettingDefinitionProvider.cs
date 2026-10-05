using System.Globalization;
using Volo.Abp.Identity.Settings;
using Volo.Abp.Settings;

namespace Dosi.Tracker.Identity;

/// <summary>
/// The workspace password policy: at least <see cref="RequiredLength"/> characters with a digit, an upper-case and a
/// lower-case letter; a symbol is not required. These are setting defaults, so a workspace (tenant) or the platform can
/// still tighten them through setting management. The owner and manager accounts this protects can read every
/// member's screenshots, webcam frames and activity, so the defaults must not accept passwords such as "111111".
/// </summary>
public class ChangeIdentityPasswordPolicySettingDefinitionProvider : SettingDefinitionProvider
{
    public const int RequiredLength = 8;

    public override void Define(ISettingDefinitionContext context)
    {
        SetDefault(context, IdentitySettingNames.Password.RequiredLength, RequiredLength.ToString(CultureInfo.InvariantCulture));
        SetDefault(context, IdentitySettingNames.Password.RequireDigit, true.ToString());
        SetDefault(context, IdentitySettingNames.Password.RequireUppercase, true.ToString());
        SetDefault(context, IdentitySettingNames.Password.RequireLowercase, true.ToString());
        SetDefault(context, IdentitySettingNames.Password.RequireNonAlphanumeric, false.ToString());
    }

    private static void SetDefault(ISettingDefinitionContext context, string name, string value)
    {
        var definition = context.GetOrNull(name);
        if (definition != null)
        {
            definition.DefaultValue = value;
        }
    }
}
