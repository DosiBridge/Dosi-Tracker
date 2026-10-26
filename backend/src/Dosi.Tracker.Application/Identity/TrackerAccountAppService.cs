using System.Threading.Tasks;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Account;
using Volo.Abp.Account.Emailing;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Identity;

namespace Dosi.Tracker.Identity;

/// <summary>
/// ABP's account service with account recovery that gives nothing away: <c>POST /api/account/send-password-reset-code</c>
/// answers the same (204) whether or not the address has an account, and an inactive account is sent nothing. A
/// password reset completed through the emailed link also confirms the address, since only its owner could open it —
/// which is how the real owner of an address someone else registered first takes the account back. Served only by ABP's
/// account controller (<c>/api/account</c>), never under a second, conventional route.
/// </summary>
[RemoteService(IsEnabled = false)]
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(IAccountAppService), typeof(AccountAppService), typeof(TrackerAccountAppService))]
public class TrackerAccountAppService : AccountAppService
{
    public TrackerAccountAppService(
        IdentityUserManager userManager,
        IIdentityRoleRepository roleRepository,
        IAccountEmailer accountEmailer,
        IdentitySecurityLogManager identitySecurityLogManager,
        IOptions<IdentityOptions> identityOptions)
        : base(userManager, roleRepository, accountEmailer, identitySecurityLogManager, identityOptions)
    {
    }

    public override async Task SendPasswordResetCodeAsync(SendPasswordResetCodeDto input)
    {
        var user = await UserManager.FindByEmailAsync(input.Email);
        if (user == null || !user.IsActive)
        {
            Logger.LogInformation("A password reset was requested for an address without an active account; nothing was sent.");
            return;
        }

        var resetToken = await UserManager.GeneratePasswordResetTokenAsync(user);
        await AccountEmailer.SendPasswordResetLinkAsync(user, resetToken, input.AppName, input.ReturnUrl, input.ReturnUrlHash);
    }

    public override async Task ResetPasswordAsync(ResetPasswordDto input)
    {
        await base.ResetPasswordAsync(input);

        var user = await UserManager.GetByIdAsync(input.UserId);
        if (!user.EmailConfirmed)
        {
            user.SetEmailConfirmed(true);
            (await UserManager.UpdateAsync(user)).CheckErrors();
        }
    }
}
