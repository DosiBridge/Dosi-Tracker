using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Security.Claims;
using System.Security.Principal;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.TestHost;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using OpenIddict.Abstractions;
using OpenIddict.Server;
using Shouldly;
using Volo.Abp;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Identity;
using Volo.Abp.Identity.AspNetCore;
using Volo.Abp.Modularity;
using Volo.Abp.OpenIddict;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>
/// The agent sign-in of docs/agent-api-contract.md against ABP's real token endpoint and OpenIddict server with the
/// host's device handlers: a password grant with device_id gets tokens bound to the install, a refresh naming another
/// install or a malformed id is refused, revoking the device makes its refresh token fail with invalid_grant, and
/// RFC 7009 revocation at /connect/revoke ends a refresh token on sign-out.
/// </summary>
public class DeviceBoundSignInEndToEndTests : TrackerEntityFrameworkCoreTestBase
{
    private const string ClientId = "Tracker_App";
    private const string UserName = "agent_e2e";

    // A synthetic account of the in-memory test database, not a credential of any deployment.
    private const string Password = "Synthetic-Agent-1!";

    private static readonly Guid UserId = Guid.Parse("cf701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Laptop = Guid.Parse("cf701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Desktop = Guid.Parse("cf701e62-0953-4dd3-910b-dc6cc93ccb03");

    private async Task<IHost> StartServerAsync()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            if (await users.FindByIdAsync(UserId.ToString()) == null)
            {
                (await users.CreateAsync(new IdentityUser(UserId, UserName, UserName + "@test.com"), Password)).Succeeded.ShouldBeTrue();
            }

            var scopes = GetRequiredService<IOpenIddictScopeManager>();
            if (await scopes.FindByNameAsync("Tracker") == null)
            {
                await scopes.CreateAsync(new OpenIddictScopeDescriptor { Name = "Tracker", Resources = { "Tracker" } });
            }

            var applications = GetRequiredService<IOpenIddictApplicationManager>();
            if (await applications.FindByClientIdAsync(ClientId) == null)
            {
                await applications.CreateAsync(new OpenIddictApplicationDescriptor
                {
                    ClientId = ClientId,
                    ClientType = OpenIddictConstants.ClientTypes.Public,
                    Permissions =
                    {
                        OpenIddictConstants.Permissions.Endpoints.Token,
                        OpenIddictConstants.Permissions.Endpoints.Revocation,
                        OpenIddictConstants.Permissions.GrantTypes.Password,
                        OpenIddictConstants.Permissions.GrantTypes.RefreshToken,
                        OpenIddictConstants.Permissions.Prefixes.Scope + "Tracker"
                    }
                });
            }
        });

        var connection = GetRequiredService<TrackerDbContext>().Database.GetDbConnection();
        var configuration = GetRequiredService<Microsoft.Extensions.Configuration.IConfiguration>();
        var host = new HostBuilder()
            .UseAutofac()
            .ConfigureWebHost(web => web
                .UseTestServer()
                .ConfigureServices(services =>
                {
                    services.ReplaceConfiguration(configuration);
                    services.AddLogging();
                    services.Configure<AbpBackgroundWorkerOptions>(options => options.IsEnabled = false);
                    services.AddApplication<DeviceBoundSignInTestModule>();
                    services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
                        options.Configure(ctx => ctx.DbContextOptions.UseSqlite(connection)));
                })
                .Configure(app =>
                {
                    app.InitializeApplication();
                    app.UseRouting();
                    app.UseAuthentication();
                    app.UseUnitOfWork();
                    app.UseAuthorization();
                    app.UseEndpoints(endpoints => endpoints.MapControllers());
                }))
            .Build();

        await host.StartAsync();
        return host;
    }

    private static async Task<(HttpStatusCode Status, JsonElement Body)> PostFormAsync(HttpClient client, string path, Dictionary<string, string> form)
    {
        var response = await client.PostAsync(path, new FormUrlEncodedContent(form));
        var text = await response.Content.ReadAsStringAsync();
        return (response.StatusCode, text.Length == 0 ? default : JsonDocument.Parse(text).RootElement.Clone());
    }

    private static Task<(HttpStatusCode Status, JsonElement Body)> PasswordAsync(HttpClient client, string? deviceId)
    {
        var form = new Dictionary<string, string>
        {
            ["grant_type"] = "password",
            ["client_id"] = ClientId,
            ["username"] = UserName,
            ["password"] = Password,
            ["scope"] = "Tracker offline_access"
        };
        if (deviceId != null)
        {
            form[DeviceConsts.TokenRequestParameter] = deviceId;
        }

        return PostFormAsync(client, "/connect/token", form);
    }

    private static Task<(HttpStatusCode Status, JsonElement Body)> RefreshAsync(HttpClient client, string refreshToken, string? deviceId)
    {
        var form = new Dictionary<string, string>
        {
            ["grant_type"] = "refresh_token",
            ["client_id"] = ClientId,
            ["refresh_token"] = refreshToken
        };
        if (deviceId != null)
        {
            form[DeviceConsts.TokenRequestParameter] = deviceId;
        }

        return PostFormAsync(client, "/connect/token", form);
    }

    /// <summary>The claims of a JWT access token (ABP issues them unencrypted).</summary>
    private static JsonElement AccessTokenClaims(JsonElement tokenResponse)
    {
        var payload = tokenResponse.GetProperty("access_token").GetString()!.Split('.')[1].Replace('-', '+').Replace('_', '/');
        payload = payload.PadRight(payload.Length + (4 - payload.Length % 4) % 4, '=');
        return JsonDocument.Parse(Encoding.UTF8.GetString(Convert.FromBase64String(payload))).RootElement.Clone();
    }

    private static async Task RevokeDeviceInServerAsync(IHost host)
    {
        // The server's own services: OpenIddict's entity cache is per container.
        using var scope = host.Services.CreateScope();
        using var uow = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>().Begin(requiresNew: true);
        (await scope.ServiceProvider.GetRequiredService<DeviceSessionManager>().RevokeAsync(UserId, Laptop)).ShouldBe(1);
        await uow.CompleteAsync();
    }

    [Fact]
    public async Task An_Agent_Sign_In_Is_Bound_To_Its_Install_And_Revoking_The_Install_Ends_Its_Refresh_Token()
    {
        using var host = await StartServerAsync();
        var client = host.GetTestClient();

        var signIn = await PasswordAsync(client, Laptop.ToString());
        signIn.Status.ShouldBe(HttpStatusCode.OK, signIn.Body.ToString());
        AccessTokenClaims(signIn.Body).GetProperty(DeviceConsts.TokenClaimType).GetString().ShouldBe(Laptop.ToString());
        var refreshToken = signIn.Body.GetProperty("refresh_token").GetString()!;

        // Another install cannot redeem this refresh token.
        var stolen = await RefreshAsync(client, refreshToken, Desktop.ToString());
        stolen.Status.ShouldBe(HttpStatusCode.BadRequest);
        stolen.Body.GetProperty("error").GetString().ShouldBe(OpenIddictConstants.Errors.InvalidGrant);

        // The install itself refreshes, and the new tokens keep its device.
        var refreshed = await RefreshAsync(client, refreshToken, Laptop.ToString());
        refreshed.Status.ShouldBe(HttpStatusCode.OK, refreshed.Body.ToString());
        AccessTokenClaims(refreshed.Body).GetProperty(DeviceConsts.TokenClaimType).GetString().ShouldBe(Laptop.ToString());
        var rolled = refreshed.Body.GetProperty("refresh_token").GetString()!;
        var withoutDeviceId = await RefreshAsync(client, rolled, null);
        withoutDeviceId.Status.ShouldBe(HttpStatusCode.OK, withoutDeviceId.Body.ToString());
        AccessTokenClaims(withoutDeviceId.Body).GetProperty(DeviceConsts.TokenClaimType).GetString().ShouldBe(Laptop.ToString());
        var latest = withoutDeviceId.Body.GetProperty("refresh_token").GetString()!;

        // A web sign-in (no device_id) is not device-bound and survives the device's revocation.
        var web = await PasswordAsync(client, null);
        web.Status.ShouldBe(HttpStatusCode.OK, web.Body.ToString());
        AccessTokenClaims(web.Body).TryGetProperty(DeviceConsts.TokenClaimType, out _).ShouldBeFalse();

        await RevokeDeviceInServerAsync(host);

        var afterRevocation = await RefreshAsync(client, latest, Laptop.ToString());
        afterRevocation.Status.ShouldBe(HttpStatusCode.BadRequest);
        afterRevocation.Body.GetProperty("error").GetString().ShouldBe(OpenIddictConstants.Errors.InvalidGrant);

        var webRefresh = await RefreshAsync(client, web.Body.GetProperty("refresh_token").GetString()!, null);
        webRefresh.Status.ShouldBe(HttpStatusCode.OK, webRefresh.Body.ToString());
    }

    [Theory]
    [InlineData("laptop")]
    [InlineData("00000000-0000-0000-0000-000000000000")]
    public async Task A_Malformed_Device_Id_Is_Refused_As_An_Invalid_Request(string deviceId)
    {
        using var host = await StartServerAsync();

        var response = await PasswordAsync(host.GetTestClient(), deviceId);

        response.Status.ShouldBe(HttpStatusCode.BadRequest);
        response.Body.GetProperty("error").GetString().ShouldBe(OpenIddictConstants.Errors.InvalidRequest);
    }

    [Theory]
    [InlineData("/connect/revoke")]
    [InlineData("/connect/revocat")]
    public async Task Signing_Out_Revokes_The_Refresh_Token_At_The_Revocation_Endpoint(string path)
    {
        using var host = await StartServerAsync();
        var client = host.GetTestClient();
        var signIn = await PasswordAsync(client, Laptop.ToString());
        var refreshToken = signIn.Body.GetProperty("refresh_token").GetString()!;

        var revoked = await PostFormAsync(client, path, new Dictionary<string, string>
        {
            ["client_id"] = ClientId,
            ["token"] = refreshToken,
            ["token_type_hint"] = "refresh_token"
        });
        revoked.Status.ShouldBe(HttpStatusCode.OK);

        var refresh = await RefreshAsync(client, refreshToken, Laptop.ToString());
        refresh.Status.ShouldBe(HttpStatusCode.BadRequest);
        refresh.Body.GetProperty("error").GetString().ShouldBe(OpenIddictConstants.Errors.InvalidGrant);
    }
}

/// <summary>The test server of <see cref="DeviceBoundSignInEndToEndTests"/>: the tracker's HTTP API test module plus ABP's
/// OpenIddict server and token endpoint, configured like the host (TrackerHttpApiHostModule) for the agent sign-in.</summary>
[DependsOn(
    typeof(TrackerHttpApiServerTestModule),
    typeof(AbpIdentityAspNetCoreModule),
    typeof(AbpOpenIddictAspNetCoreModule))]
public class DeviceBoundSignInTestModule : AbpModule
{
    public override void PreConfigureServices(ServiceConfigurationContext context)
    {
        // The token endpoint's controller (a web project gets it as a generated application part).
        PreConfigure<IMvcBuilder>(mvcBuilder =>
            mvcBuilder.AddApplicationPartIfNotExists(typeof(AbpOpenIddictAspNetCoreModule).Assembly));

        PreConfigure<AbpOpenIddictAspNetCoreOptions>(options =>
        {
            options.AddDevelopmentEncryptionAndSigningCertificate = false;
            // ABP would switch the process-wide claim types to OpenIddict's, under every other test of this assembly;
            // SubjectClaimContributor adds the "sub" claim OpenIddict needs instead.
            options.UpdateAbpClaimTypes = false;
        });

        PreConfigure<OpenIddictServerBuilder>(builder =>
        {
            builder.AddEphemeralEncryptionKey().AddEphemeralSigningKey();
            builder.UseAspNetCore().DisableTransportSecurityRequirement();

            // As TrackerHttpApiHostModule configures them.
            builder.AddEventHandler(ValidateDeviceIdParameter.Descriptor);
            builder.AddEventHandler(AttachDeviceBinding.Descriptor);
            builder.SetRevocationEndpointUris("connect/revocat", "connect/revoke");
        });
    }

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        context.Services.AddTransient<SubjectClaimContributor>();
        Configure<AbpClaimsPrincipalFactoryOptions>(options =>
        {
            options.Contributors.Add<SubjectClaimContributor>();
            options.IsDynamicClaimsEnabled = true;
        });
    }
}

/// <summary>Adds OpenIddict's subject claim to a signed-in user's principal (the host gets it from ABP's claim types).</summary>
[DisableConventionalRegistration]
public class SubjectClaimContributor : IAbpClaimsPrincipalContributor
{
    public Task ContributeAsync(AbpClaimsPrincipalContributorContext context)
    {
        var identity = context.ClaimsPrincipal.Identities.First();
        var userId = identity.FindUserId();
        if (userId.HasValue && !identity.HasClaim(c => c.Type == OpenIddictConstants.Claims.Subject))
        {
            identity.AddClaim(new Claim(OpenIddictConstants.Claims.Subject, userId.Value.ToString()));
        }

        return Task.CompletedTask;
    }
}
