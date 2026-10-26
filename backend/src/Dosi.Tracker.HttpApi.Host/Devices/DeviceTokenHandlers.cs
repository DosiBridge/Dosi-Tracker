using System.Threading.Tasks;
using OpenIddict.Abstractions;
using OpenIddict.Server;
using static OpenIddict.Server.OpenIddictServerEvents;

namespace Dosi.Tracker.Devices;

/// <summary>
/// Binds the tokens of a desktop agent sign-in to its install (<see cref="DeviceTokenBinding"/>). A password or
/// refresh-token request may carry <c>device_id</c>: a malformed one is refused (<c>invalid_request</c>), and a
/// refresh naming another device than its refresh token was issued to is refused (<c>invalid_grant</c>), so a copied
/// refresh token cannot be relabelled as another install.
/// </summary>
public sealed class ValidateDeviceIdParameter : IOpenIddictServerHandler<ValidateTokenRequestContext>
{
    /// <summary>Runs once the refresh token being redeemed is validated and its principal is known.</summary>
    public static OpenIddictServerHandlerDescriptor Descriptor { get; } =
        OpenIddictServerHandlerDescriptor.CreateBuilder<ValidateTokenRequestContext>()
            .UseSingletonHandler<ValidateDeviceIdParameter>()
            .SetOrder(OpenIddictServerHandlers.Exchange.ValidateAuthentication.Descriptor.Order + 500)
            .SetType(OpenIddictServerHandlerType.Custom)
            .Build();

    public ValueTask HandleAsync(ValidateTokenRequestContext context)
    {
        if (!context.Request.IsPasswordGrantType() && !context.Request.IsRefreshTokenGrantType())
        {
            return default;
        }

        var decision = DeviceTokenBinding.Decide(
            (string?)context.Request.GetParameter(DeviceConsts.TokenRequestParameter),
            context.RefreshTokenPrincipal?.GetClaim(DeviceConsts.TokenClaimType));

        switch (decision.Outcome)
        {
            case DeviceTokenBinding.Outcome.InvalidDeviceId:
                context.Reject(
                    OpenIddictConstants.Errors.InvalidRequest,
                    $"The {DeviceConsts.TokenRequestParameter} parameter must be a UUID.");
                break;
            case DeviceTokenBinding.Outcome.DeviceMismatch:
                context.Reject(
                    OpenIddictConstants.Errors.InvalidGrant,
                    "The refresh token was issued to another device.");
                break;
        }

        return default;
    }
}

/// <summary>
/// Puts the device of a password or refresh-token sign-in into its tokens (<see cref="DeviceConsts.TokenClaimType"/>,
/// in the access token too) and records it on the sign-in's OpenIddict authorization, which every refresh token of
/// the sign-in belongs to, so that revoking the device revokes them (<see cref="DeviceSessionManager"/>). Runs after
/// OpenIddict attached the authorization and before it builds the tokens.
/// </summary>
public sealed class AttachDeviceBinding : IOpenIddictServerHandler<ProcessSignInContext>
{
    private readonly DeviceSessionManager _sessionManager;

    public AttachDeviceBinding(DeviceSessionManager sessionManager)
    {
        _sessionManager = sessionManager;
    }

    public static OpenIddictServerHandlerDescriptor Descriptor { get; } =
        OpenIddictServerHandlerDescriptor.CreateBuilder<ProcessSignInContext>()
            .UseScopedHandler<AttachDeviceBinding>()
            .SetOrder(OpenIddictServerHandlers.AttachAuthorization.Descriptor.Order + 500)
            .SetType(OpenIddictServerHandlerType.Custom)
            .Build();

    public async ValueTask HandleAsync(ProcessSignInContext context)
    {
        if (context.EndpointType != OpenIddictServerEndpointType.Token ||
            context.Principal == null ||
            (!context.Request.IsPasswordGrantType() && !context.Request.IsRefreshTokenGrantType()))
        {
            return;
        }

        // Already validated (ValidateDeviceIdParameter): the refresh token's device wins, else the request's.
        var decision = DeviceTokenBinding.Decide(
            (string?)context.Request.GetParameter(DeviceConsts.TokenRequestParameter),
            context.Principal.GetClaim(DeviceConsts.TokenClaimType));
        if (decision is not { Outcome: DeviceTokenBinding.Outcome.Bound, DeviceId: { } deviceId })
        {
            return;
        }

        context.Principal.SetClaim(DeviceConsts.TokenClaimType, DeviceTokenBinding.Format(deviceId));
        foreach (var claim in context.Principal.Claims)
        {
            if (claim.Type == DeviceConsts.TokenClaimType)
            {
                claim.SetDestinations(OpenIddictConstants.Destinations.AccessToken);
            }
        }

        var authorizationId = context.Principal.GetAuthorizationId();
        if (string.IsNullOrEmpty(authorizationId))
        {
            return;
        }

        // A brand-new sign-in's authorization can still be mid-write elsewhere in OpenIddict's own pipeline (it
        // was only just created a few handlers earlier) when BindAsync reads it, so its first attempt can lose a
        // concurrency race against that write. Re-reading and retrying — same as any optimistic-concurrency
        // caller — picks up the now-current row; it never retries a real rejection (BindAsync returns true as
        // soon as the row already carries a device, including the one this call tried to write).
        for (var attempt = 1; attempt <= MaxBindAttempts; attempt++)
        {
            if (await _sessionManager.BindAsync(authorizationId, deviceId))
            {
                return;
            }
        }
    }

    private const int MaxBindAttempts = 3;
}
