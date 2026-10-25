namespace Dosi.Tracker.Devices;

public static class DeviceConsts
{
    public const int MaxReasonLength = 256;
    public const int MaxRevocationsListing = 100;

    /// <summary>Most devices <c>GET /api/app/device</c> lists. Presence keeps at most a handful of devices per member
    /// (<c>AgentPresenceConsts.MaxDevicesPerUser</c>), so this only bounds a very large workspace.</summary>
    public const int MaxDevicesListing = 1000;

    /// <summary>The form field of a password or refresh-token request (<c>POST /connect/token</c>) naming the agent
    /// install the tokens are for: the random UUID the agent generated once and persists.</summary>
    public const string TokenRequestParameter = "device_id";

    /// <summary>The claim of the access and refresh tokens that carries the install they were issued to.</summary>
    public const string TokenClaimType = "device_id";

    /// <summary>The property of an OpenIddict authorization recording the install its tokens were issued to, so that
    /// revoking the device can find and revoke them.</summary>
    public const string AuthorizationProperty = "dosi_device_id";
}
