using Volo.Abp.Account.Settings;
using Volo.Abp.Identity.Settings;
using Volo.Abp.Settings;

namespace Dosi.Tracker.Settings;

/// <summary>
/// Turns ABP's anonymous self-registration (<c>/api/account/register</c> and the MVC register page) off by default:
/// accounts are created by <c>WorkspaceAppService.RegisterAsync</c>. Also stops users from changing their email in
/// their profile: invitations and workspace membership are matched by email, so an address must not be claimable later.
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

        var emailUpdate = context.GetOrNull(IdentitySettingNames.User.IsEmailUpdateEnabled);
        if (emailUpdate != null)
        {
            emailUpdate.DefaultValue = false.ToString();
        }
    }
}
