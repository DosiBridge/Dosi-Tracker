using System;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.EntityFrameworkCore;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;
using OpenIddict.Abstractions;
using OpenIddict.Server.AspNetCore;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.EntityFrameworkCore;
using Volo.Abp.Identity;
using Volo.Abp.OpenIddict;
using Volo.Abp.OpenIddict.Controllers;
using Volo.Abp.Security.Claims;

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
/// the base method exactly, except for the last step. ABP's own recommended interim fix — recording the sign-
/// in time in a separate, immediately-completing unit of work — was tried first here and confirmed, against a
/// real deploy, to deadlock instead: that separate transaction waits on the row lock the request's OWN still-
/// open outer transaction already holds on this same user (from the access-failed-count reset above), and
/// never gets it until that outer transaction ends — which only happens after this method returns. Recording
/// the timestamp with a raw, targeted UPDATE instead avoids every part of that: it runs on the request's own
/// connection and transaction (no second transaction to deadlock against), and it never touches EF Core's
/// change tracker or its concurrency-token check at all, so it cannot raise the conflict ABP's bug report
/// describes by construction. "Last signed in at" is a last-write-wins field by nature — a second, genuinely
/// concurrent sign-in simply overwriting it a moment later is correct, not a bug to guard against.
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
        var dbContext = await LazyServiceProvider
            .LazyGetRequiredService<IDbContextProvider<TrackerDbContext>>()
            .GetDbContextAsync();

        await dbContext.Database.ExecuteSqlInterpolatedAsync(
            $"UPDATE \"AbpUsers\" SET \"LastSignInTime\" = {DateTimeOffset.UtcNow} WHERE \"Id\" = {userId}");
    }
}
