using System;

namespace Dosi.Tracker.Devices;

/// <summary>
/// The rules binding a desktop agent install (its device id) to the tokens it signs in with. A password grant that
/// sends <see cref="DeviceConsts.TokenRequestParameter"/> gets tokens carrying the device
/// (<see cref="DeviceConsts.TokenClaimType"/>); a refresh keeps the device of the refresh token, and may only name
/// that same device. A refresh token issued before devices were bound is bound by the first refresh naming one.
/// </summary>
public static class DeviceTokenBinding
{
    public enum Outcome
    {
        /// <summary>No device is named or bound: the tokens are not device-bound (the web dashboard).</summary>
        Unbound,

        /// <summary>The tokens are bound to <see cref="Decision.DeviceId"/>.</summary>
        Bound,

        /// <summary>The request's device id is not a UUID (or is the empty one): <c>invalid_request</c>.</summary>
        InvalidDeviceId,

        /// <summary>The refresh token was issued to another device than the request names: <c>invalid_grant</c>.</summary>
        DeviceMismatch
    }

    public readonly record struct Decision(Outcome Outcome, Guid? DeviceId);

    /// <summary>A non-empty device id, or null.</summary>
    public static Guid? Parse(string? value) =>
        Guid.TryParse(value?.Trim(), out var id) && id != Guid.Empty ? id : null;

    /// <summary>The canonical form stored in claims and authorization properties.</summary>
    public static string Format(Guid deviceId) => deviceId.ToString("D");

    /// <summary>
    /// What a token request yields: <paramref name="requested"/> is the request's
    /// <see cref="DeviceConsts.TokenRequestParameter"/> (null when absent), <paramref name="bound"/> the device claim
    /// of the refresh token being redeemed (null for a password grant or an unbound refresh token).
    /// </summary>
    public static Decision Decide(string? requested, string? bound)
    {
        Guid? requestedId = null;
        if (!string.IsNullOrWhiteSpace(requested))
        {
            requestedId = Parse(requested);
            if (requestedId == null)
            {
                return new Decision(Outcome.InvalidDeviceId, null);
            }
        }

        var boundId = Parse(bound);
        if (boundId != null)
        {
            return requestedId != null && requestedId != boundId
                ? new Decision(Outcome.DeviceMismatch, boundId)
                : new Decision(Outcome.Bound, boundId);
        }

        return requestedId != null
            ? new Decision(Outcome.Bound, requestedId)
            : new Decision(Outcome.Unbound, null);
    }
}
