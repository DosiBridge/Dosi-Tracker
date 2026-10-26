using System;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Mvc;
using Microsoft.Extensions.Logging;
using OpenIddict.Abstractions;
using OpenIddict.Server.AspNetCore;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Identity;
using Volo.Abp.OpenIddict;
using Volo.Abp.OpenIddict.Controllers;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Replaces ABP 10.5's <see cref="TokenController"/> to work around a framework bug the ABP team has confirmed
/// and said they will fix (ABP support question #10828, "Concurrent logins cause AbpDbConcurrencyException
/// updating LastSignInTime"): <c>SetSuccessResultAsync</c> sets the signed-in user's last-sign-in time and
/// saves it on the request's own tracked <see cref="IdentityUser"/>, inside the request's ambient unit of
/// work, which only actually commits at the very end of the MVC action (<c>AbpUowActionFilter</c>). Anything
/// else that updates and saves that same user first — including ASP.NET Core Identity's own access-failed-
/// count reset after a prior wrong password, i.e. exactly what happens right after a worker or agent retries a
/// mistyped password and then gets it right — bumps the row's concurrency stamp before that final save runs,
/// so the whole sign-in fails with 409 even though the credentials were correct. Confirmed end to end against
/// a real deploy.
///
/// Every grant type (password, refresh, authorization code, device code, client credentials) funnels into this
/// one shared method for its final steps, so overriding only it covers all of them. The override reproduces
/// the base method exactly, except for the last step: recording the sign-in time now happens in its own,
/// separate, immediately-completing unit of work against a freshly reloaded user, with exactly this one known
/// concurrency conflict caught and ignored — the ABP team's own recommended interim fix. A lost race there
/// costs only a "last signed in at" timestamp; it must never cost the sign-in itself.
///
/// Remove this override once the fix lands in the ABP framework (tracked upstream in support question #10828).
/// </summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(TokenController))]
public class TrackerTokenController : TokenController
{
    protected override async Task<IActionResult> SetSuccessResultAsync(OpenIddictRequest request, IdentityUser user)
    {
        // A new ClaimsPrincipal containing the claims to be used to create the id_token, access_token or code.
        await IdentityDynamicClaimsPrincipalContributorCache.ClearAsync(user.Id, user.TenantId);

        var principal = await SignInManager.CreateUserPrincipalAsync(user);

        var rememberMe = request.GetParameter("RememberMe").ToString();
        if (!rememberMe.IsNullOrWhiteSpace() && bool.TryParse(rememberMe, out var rememberMeValue) && rememberMeValue)
        {
            var claim = new Claim(AbpClaimTypes.RememberMe, true.ToString())
                .SetDestinations(OpenIddictConstants.Destinations.AccessToken);
            principal.Identities.FirstOrDefault()?.AddClaim(claim);
        }

        principal.SetScopes(request.GetScopes());
        principal.SetResources(await GetResourcesAsync(request.GetScopes()));

        await OpenIddictClaimsPrincipalManager.HandleAsync(request, principal);

        await IdentitySecurityLogManager.SaveAsync(new IdentitySecurityLogContext
        {
            Identity = OpenIddictSecurityLogIdentityConsts.OpenIddict,
            Action = OpenIddictSecurityLogActionConsts.LoginSucceeded,
            UserName = request.Username,
            ClientId = request.ClientId,
        });

        // The only change from the base method — see the class summary (ABP support #10828).
        await RecordLastSignInTimeAsync(user.Id);

        return SignIn(principal, OpenIddictServerAspNetCoreDefaults.AuthenticationScheme);
    }

    private async Task RecordLastSignInTimeAsync(Guid userId)
    {
        try
        {
            using var uow = LazyServiceProvider.LazyGetRequiredService<IUnitOfWorkManager>().Begin(requiresNew: true);

            var freshUser = await UserManager.GetByIdAsync(userId);
            freshUser.SetLastSignInTime(DateTimeOffset.UtcNow);
            await UserManager.UpdateAsync(freshUser);

            await uow.CompleteAsync();
        }
        catch (AbpDbConcurrencyException ex)
        {
            Logger.LogInformation(
                "Could not record the last sign-in time for user {UserId}: {Message} (the sign-in still succeeds).",
                userId,
                ex.Message);
        }
    }
}
