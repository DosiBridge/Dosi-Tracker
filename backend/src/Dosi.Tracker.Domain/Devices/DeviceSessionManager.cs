using System;
using System.Collections.Generic;
using System.Collections.Immutable;
using System.Text.Json;
using System.Threading.Tasks;
using OpenIddict.Abstractions;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker.Devices;

/// <summary>
/// The sign-in sessions of a desktop agent install. A password sign-in that names its device
/// (<see cref="DeviceConsts.TokenRequestParameter"/>) gets an OpenIddict authorization recording the device
/// (<see cref="DeviceConsts.AuthorizationProperty"/>); every refresh token of that sign-in belongs to it. Revoking
/// the device revokes those authorizations and their tokens, so the install's refresh token is refused
/// (<c>invalid_grant</c>) and it cannot continue without the member's password. Sessions that named no device (the
/// web dashboard, agents older than the device-bound sign-in) are never touched.
/// </summary>
public class DeviceSessionManager : DomainService
{
    private readonly IOpenIddictAuthorizationManager _authorizationManager;
    private readonly IOpenIddictTokenManager _tokenManager;

    public DeviceSessionManager(
        IOpenIddictAuthorizationManager authorizationManager,
        IOpenIddictTokenManager tokenManager)
    {
        _authorizationManager = authorizationManager;
        _tokenManager = tokenManager;
    }

    /// <summary>Records <paramref name="deviceId"/> on the authorization <paramref name="authorizationId"/> (written
    /// only when it is not recorded yet). An authorization of another device keeps its device: the token request
    /// was refused before it could reach this point.</summary>
    public virtual async Task BindAsync(string authorizationId, Guid deviceId)
    {
        var authorization = await _authorizationManager.FindByIdAsync(authorizationId);
        if (authorization == null)
        {
            return;
        }

        var properties = await _authorizationManager.GetPropertiesAsync(authorization);
        if (FindDevice(properties) != null)
        {
            return;
        }

        var descriptor = new OpenIddictAuthorizationDescriptor();
        await _authorizationManager.PopulateAsync(descriptor, authorization);
        descriptor.Properties[DeviceConsts.AuthorizationProperty] =
            JsonSerializer.SerializeToElement(DeviceTokenBinding.Format(deviceId));
        await _authorizationManager.UpdateAsync(authorization, descriptor);
    }

    /// <summary>The device recorded on <paramref name="authorizationId"/>, or null (unknown or unbound).</summary>
    public virtual async Task<Guid?> FindBoundDeviceAsync(string authorizationId)
    {
        var authorization = await _authorizationManager.FindByIdAsync(authorizationId);
        return authorization == null
            ? null
            : FindDevice(await _authorizationManager.GetPropertiesAsync(authorization));
    }

    /// <summary>Revokes every authorization of <paramref name="userId"/> bound to <paramref name="deviceId"/>, and
    /// all their tokens. Returns how many authorizations were revoked (already revoked ones are revoked again, which
    /// changes nothing).</summary>
    public virtual async Task<int> RevokeAsync(Guid userId, Guid deviceId)
    {
        var bound = new List<(object Authorization, string Id)>();
        await foreach (var authorization in _authorizationManager.FindBySubjectAsync(userId.ToString()))
        {
            if (FindDevice(await _authorizationManager.GetPropertiesAsync(authorization)) != deviceId)
            {
                continue;
            }

            var id = await _authorizationManager.GetIdAsync(authorization);
            if (!string.IsNullOrEmpty(id))
            {
                bound.Add((authorization, id));
            }
        }

        foreach (var (authorization, id) in bound)
        {
            await RevokeUsableTokensAsync(id);
            await _authorizationManager.TryRevokeAsync(authorization);
        }

        return bound.Count;
    }

    /// <summary>Revokes the tokens of an authorization that could still be used: valid and not expired (a redeemed
    /// refresh token is already refused, and presenting it again makes OpenIddict revoke the whole chain). One by one
    /// through the manager rather than in bulk, because the manager also refreshes OpenIddict's entity cache, which a
    /// bulk update bypasses: a cached token would otherwise still look valid when the install next refreshes.</summary>
    private async Task RevokeUsableTokensAsync(string authorizationId)
    {
        var now = DateTimeOffset.UtcNow;
        var usable = new List<object>();
        await foreach (var token in _tokenManager.FindByAuthorizationIdAsync(authorizationId))
        {
            var expires = await _tokenManager.GetExpirationDateAsync(token);
            if (await _tokenManager.HasStatusAsync(token, OpenIddictConstants.Statuses.Valid) &&
                (expires == null || expires.Value > now))
            {
                usable.Add(token);
            }
        }

        foreach (var token in usable)
        {
            await _tokenManager.TryRevokeAsync(token);
        }
    }

    /// <summary>The device recorded in an authorization's properties, or null.</summary>
    public static Guid? FindDevice(ImmutableDictionary<string, JsonElement> properties)
    {
        return properties.TryGetValue(DeviceConsts.AuthorizationProperty, out var value) &&
               value.ValueKind == JsonValueKind.String
            ? DeviceTokenBinding.Parse(value.GetString())
            : null;
    }
}
