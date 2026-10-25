using System;
using System.Linq;
using System.Security.Claims;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Microsoft.Extensions.Logging.Abstractions;
using OpenIddict.Abstractions;
using OpenIddict.Server;
using Shouldly;
using Xunit;
using static OpenIddict.Server.OpenIddictServerEvents;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>
/// The OpenIddict handlers that bind a desktop agent's tokens to its install, run against the real OpenIddict stores:
/// a malformed device_id or a refresh naming another device is refused, and a password or refresh sign-in naming a
/// device puts it in the access token and records it on the sign-in's authorization (which revoking the device uses).
/// </summary>
public class DeviceTokenHandlersTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly Guid UserId = Guid.Parse("9d701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Laptop = Guid.Parse("9d701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Desktop = Guid.Parse("9d701e62-0953-4dd3-910b-dc6cc93ccb03");

    private static OpenIddictServerTransaction Transaction(OpenIddictRequest request, OpenIddictServerEndpointType endpoint = OpenIddictServerEndpointType.Token) => new()
    {
        EndpointType = endpoint,
        Request = request,
        Options = new OpenIddictServerOptions(),
        Logger = NullLogger.Instance
    };

    private static OpenIddictRequest PasswordRequest(string? deviceId)
    {
        var request = new OpenIddictRequest { GrantType = OpenIddictConstants.GrantTypes.Password, Username = "agent", Password = "irrelevant" };
        if (deviceId != null)
        {
            request.SetParameter(DeviceConsts.TokenRequestParameter, deviceId);
        }

        return request;
    }

    private static OpenIddictRequest RefreshRequest(string? deviceId)
    {
        var request = new OpenIddictRequest { GrantType = OpenIddictConstants.GrantTypes.RefreshToken, RefreshToken = "opaque" };
        if (deviceId != null)
        {
            request.SetParameter(DeviceConsts.TokenRequestParameter, deviceId);
        }

        return request;
    }

    private static ClaimsPrincipal Principal(Guid? boundDevice = null, string? authorizationId = null)
    {
        var identity = new ClaimsIdentity("test");
        identity.AddClaim(new Claim(OpenIddictConstants.Claims.Subject, UserId.ToString()));
        if (boundDevice.HasValue)
        {
            identity.AddClaim(new Claim(DeviceConsts.TokenClaimType, boundDevice.Value.ToString()));
        }

        var principal = new ClaimsPrincipal(identity);
        if (authorizationId != null)
        {
            principal.SetAuthorizationId(authorizationId);
        }

        return principal;
    }

    private static async Task<ValidateTokenRequestContext> ValidateAsync(OpenIddictRequest request, ClaimsPrincipal? refreshTokenPrincipal = null)
    {
        var context = new ValidateTokenRequestContext(Transaction(request)) { RefreshTokenPrincipal = refreshTokenPrincipal };
        await new ValidateDeviceIdParameter().HandleAsync(context);
        return context;
    }

    [Theory]
    [InlineData("not-a-uuid")]
    [InlineData("00000000-0000-0000-0000-000000000000")]
    public async Task A_Malformed_Device_Id_Is_Refused_As_An_Invalid_Request(string deviceId)
    {
        var password = await ValidateAsync(PasswordRequest(deviceId));
        password.IsRejected.ShouldBeTrue();
        password.Error.ShouldBe(OpenIddictConstants.Errors.InvalidRequest);

        var refresh = await ValidateAsync(RefreshRequest(deviceId), Principal(Laptop));
        refresh.IsRejected.ShouldBeTrue();
        refresh.Error.ShouldBe(OpenIddictConstants.Errors.InvalidRequest);
    }

    [Fact]
    public async Task A_Refresh_Naming_Another_Device_Than_Its_Token_Is_Refused_As_An_Invalid_Grant()
    {
        var context = await ValidateAsync(RefreshRequest(Desktop.ToString()), Principal(Laptop));

        context.IsRejected.ShouldBeTrue();
        context.Error.ShouldBe(OpenIddictConstants.Errors.InvalidGrant);
    }

    [Fact]
    public async Task Well_Formed_Or_Absent_Device_Ids_And_Other_Grants_Pass()
    {
        (await ValidateAsync(PasswordRequest(Laptop.ToString()))).IsRejected.ShouldBeFalse();
        (await ValidateAsync(PasswordRequest(null))).IsRejected.ShouldBeFalse();
        (await ValidateAsync(RefreshRequest(Laptop.ToString().ToUpperInvariant()), Principal(Laptop))).IsRejected.ShouldBeFalse();
        (await ValidateAsync(RefreshRequest(null), Principal(Laptop))).IsRejected.ShouldBeFalse();
        // A refresh token issued before devices were bound may be bound by its first refresh.
        (await ValidateAsync(RefreshRequest(Laptop.ToString()), Principal())).IsRejected.ShouldBeFalse();

        var clientCredentials = new OpenIddictRequest { GrantType = OpenIddictConstants.GrantTypes.ClientCredentials };
        clientCredentials.SetParameter(DeviceConsts.TokenRequestParameter, "not-a-uuid");
        (await ValidateAsync(clientCredentials)).IsRejected.ShouldBeFalse();
    }

    [Fact]
    public async Task A_Password_Sign_In_Naming_A_Device_Binds_Its_Access_Token_And_Authorization()
    {
        var authorizationId = await CreateAuthorizationAsync(boundDevice: null);
        var principal = Principal(authorizationId: authorizationId);

        await SignInAsync(PasswordRequest(Laptop.ToString()), principal);

        var claim = principal.Claims.Single(c => c.Type == DeviceConsts.TokenClaimType);
        claim.Value.ShouldBe(DeviceTokenBinding.Format(Laptop));
        claim.GetDestinations().ShouldBe(new[] { OpenIddictConstants.Destinations.AccessToken });
        (await BoundDeviceAsync(authorizationId)).ShouldBe(Laptop);
    }

    [Fact]
    public async Task A_Refresh_Keeps_Its_Device_And_Binds_A_Session_Issued_Before_Devices_Were_Bound()
    {
        // Bound session: the refresh token's device stays, whatever else the request says (validation refused a mismatch).
        var boundAuthorization = await CreateAuthorizationAsync(Laptop);
        var bound = Principal(Laptop, boundAuthorization);
        await SignInAsync(RefreshRequest(null), bound);
        bound.Claims.Single(c => c.Type == DeviceConsts.TokenClaimType).Value.ShouldBe(DeviceTokenBinding.Format(Laptop));
        (await BoundDeviceAsync(boundAuthorization)).ShouldBe(Laptop);

        // Legacy session (no device anywhere yet): the first refresh naming the device binds it.
        var legacyAuthorization = await CreateAuthorizationAsync(boundDevice: null);
        var legacy = Principal(authorizationId: legacyAuthorization);
        await SignInAsync(RefreshRequest(Desktop.ToString()), legacy);
        legacy.Claims.Single(c => c.Type == DeviceConsts.TokenClaimType).Value.ShouldBe(DeviceTokenBinding.Format(Desktop));
        (await BoundDeviceAsync(legacyAuthorization)).ShouldBe(Desktop);
    }

    [Fact]
    public async Task Sign_Ins_Naming_No_Device_Or_Outside_The_Token_Endpoint_Stay_Unbound()
    {
        var authorizationId = await CreateAuthorizationAsync(boundDevice: null);

        var web = Principal(authorizationId: authorizationId);
        await SignInAsync(PasswordRequest(null), web);
        web.Claims.ShouldNotContain(c => c.Type == DeviceConsts.TokenClaimType);

        var authorizeEndpoint = Principal(authorizationId: authorizationId);
        await SignInAsync(PasswordRequest(Laptop.ToString()), authorizeEndpoint, OpenIddictServerEndpointType.Authorization);
        authorizeEndpoint.Claims.ShouldNotContain(c => c.Type == DeviceConsts.TokenClaimType);

        (await BoundDeviceAsync(authorizationId)).ShouldBeNull();

        // Without an authorization (no refresh token requested) only the access token carries the device.
        var accessOnly = Principal();
        await SignInAsync(PasswordRequest(Laptop.ToString()), accessOnly);
        accessOnly.Claims.Single(c => c.Type == DeviceConsts.TokenClaimType).Value.ShouldBe(DeviceTokenBinding.Format(Laptop));
    }

    [Fact]
    public async Task Binding_Never_Moves_An_Authorization_To_Another_Device_And_Ignores_Unknown_Ones()
    {
        var authorizationId = await CreateAuthorizationAsync(Laptop);

        await WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().BindAsync(authorizationId, Desktop));
        await WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().BindAsync(Guid.NewGuid().ToString(), Desktop));

        (await BoundDeviceAsync(authorizationId)).ShouldBe(Laptop);
        (await WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().FindBoundDeviceAsync(Guid.NewGuid().ToString())))
            .ShouldBeNull();
    }

    [Fact]
    public void Only_A_String_Property_Holding_A_Device_Id_Counts_As_A_Binding()
    {
        var properties = System.Collections.Immutable.ImmutableDictionary<string, JsonElement>.Empty;
        DeviceSessionManager.FindDevice(properties).ShouldBeNull();
        DeviceSessionManager.FindDevice(properties.Add(DeviceConsts.AuthorizationProperty, JsonSerializer.SerializeToElement(42))).ShouldBeNull();
        DeviceSessionManager.FindDevice(properties.Add(DeviceConsts.AuthorizationProperty, JsonSerializer.SerializeToElement("junk"))).ShouldBeNull();
        DeviceSessionManager.FindDevice(properties.Add(DeviceConsts.AuthorizationProperty, JsonSerializer.SerializeToElement(Laptop.ToString())))
            .ShouldBe(Laptop);
    }

    private Task SignInAsync(OpenIddictRequest request, ClaimsPrincipal principal, OpenIddictServerEndpointType endpoint = OpenIddictServerEndpointType.Token)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var context = new ProcessSignInContext(Transaction(request, endpoint)) { Principal = principal };
            await new AttachDeviceBinding(GetRequiredService<DeviceSessionManager>()).HandleAsync(context);
        });
    }

    private Task<string> CreateAuthorizationAsync(Guid? boundDevice)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var manager = GetRequiredService<IOpenIddictAuthorizationManager>();
            var descriptor = new OpenIddictAuthorizationDescriptor
            {
                CreationDate = DateTimeOffset.UtcNow,
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = UserId.ToString(),
                Type = OpenIddictConstants.AuthorizationTypes.AdHoc
            };
            descriptor.Scopes.Add("Tracker");
            if (boundDevice.HasValue)
            {
                descriptor.Properties[DeviceConsts.AuthorizationProperty] =
                    JsonSerializer.SerializeToElement(DeviceTokenBinding.Format(boundDevice.Value));
            }

            return (await manager.GetIdAsync(await manager.CreateAsync(descriptor)))!;
        });
    }

    private Task<Guid?> BoundDeviceAsync(string authorizationId)
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().FindBoundDeviceAsync(authorizationId));
    }
}
