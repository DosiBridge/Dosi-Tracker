using Volo.Abp.Account.Settings;
using Volo.Abp.Settings;

namespace Dosi.Tracker.Settings;

/// <summary>
/// Turns ABP's anonymous self-registration (<c>/api/account/register</c> and the MVC register page)
/// off by default. Without this, anyone could create a user directly in the HOST side — i.e. inside
/// the platform console's tenant. Workspaces are provisioned exclusively through
/// <c>WorkspaceAppService.RegisterAsync</c>, which creates the tenant and its admin itself and does not
/// consult this setting. An operator can still re-enable it per tenant/globally via setting management.
/// Runs after ABP's AccountSettingDefinitionProvider (module dependency order), so the definition exists.
/// </summary>
public class TrackerAccountSettingDefinitionProvider : SettingDefinitionProvider
{
    public override void Define(ISettingDefinitionContext context)
    {
        var selfRegistration = context.GetOrNull(AccountSettingNames.IsSelfRegistrationEnabled);
        if (selfRegistration != null)
        {
            selfRegistration.DefaultValue = false.ToString();
        }
    }
}
