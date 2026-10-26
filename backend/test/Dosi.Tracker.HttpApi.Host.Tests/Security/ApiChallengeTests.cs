using System.Net;
using System.Text.Encodings.Web;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Identity;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// This host has no browser sign-in flow a human ever reaches (the product's login UI is the Next.js dashboard;
/// desktop agents are headless API clients), so a missing or rejected credential must answer 401/403, never a
/// redirect to an Account/Login page that does not exist for API callers — confirmed against a real Production
/// deploy for <c>GET /api/app/workspaces/mine</c> with an invalid access token (302, not 401).
///
/// With both the Identity cookie scheme and a bearer-style scheme registered, as ABP's Identity and OpenIddict
/// modules register them, this exercises <see cref="TrackerHttpApiHostModule"/>'s <c>ConfigureAuthentication</c>:
/// pointing <c>AuthenticationOptions.DefaultChallengeScheme</c> at the bearer scheme means an endpoint whose
/// <c>[Authorize]</c> names no scheme — every ABP auto-API controller — is always challenged there, never on the
/// cookie scheme (whose own events are patched too, belt-and-suspenders, but are not what this test proves).
/// </summary>
public sealed class ApiChallengeTests : IAsyncLifetime
{
    private const string BearerScheme = "Bearer";

    private IHost _host = null!;

    public async Task InitializeAsync()
    {
        _host = await new HostBuilder()
            .ConfigureWebHost(web => web
                .UseTestServer()
                .ConfigureServices(services =>
                {
                    services.AddRouting();
                    services.AddAuthorization();
                    services.AddAuthentication(IdentityConstants.ApplicationScheme)
                        .AddCookie(IdentityConstants.ApplicationScheme)
                        .AddScheme<AuthenticationSchemeOptions, BearerStandInHandler>(BearerScheme, _ => { });

                    // The two overrides TrackerHttpApiHostModule.ConfigureAuthentication applies.
                    services.Configure<AuthenticationOptions>(options =>
                    {
                        options.DefaultChallengeScheme = BearerScheme;
                    });
                    services.Configure<CookieAuthenticationOptions>(IdentityConstants.ApplicationScheme, options =>
                    {
                        options.Events.OnRedirectToLogin = ctx =>
                        {
                            ctx.Response.StatusCode = StatusCodes.Status401Unauthorized;
                            return Task.CompletedTask;
                        };
                    });
                })
                .Configure(app =>
                {
                    app.UseAuthentication();
                    app.UseRouting();
                    app.UseAuthorization();
                    app.UseEndpoints(endpoints =>
                    {
                        endpoints.MapGet("/api/app/workspace/my-list", () => "ok").RequireAuthorization();
                    });
                }))
            .StartAsync();
    }

    public async Task DisposeAsync()
    {
        await _host.StopAsync();
        _host.Dispose();
    }

    [Fact]
    public async Task No_Credential_Is_Challenged_On_The_Bearer_Scheme_Not_The_Cookie()
    {
        using var client = _host.GetTestServer().CreateClient();

        var response = await client.GetAsync("/api/app/workspace/my-list");

        response.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
    }

    /// <summary>Stands in for OpenIddict's bearer validation handler: no principal, no redirect, just 401 — exactly
    /// what every real bearer/API authentication handler does on challenge.</summary>
    private sealed class BearerStandInHandler : AuthenticationHandler<AuthenticationSchemeOptions>
    {
        public BearerStandInHandler(
            IOptionsMonitor<AuthenticationSchemeOptions> options, ILoggerFactory logger, UrlEncoder encoder)
            : base(options, logger, encoder)
        {
        }

        protected override Task<AuthenticateResult> HandleAuthenticateAsync() =>
            Task.FromResult(AuthenticateResult.NoResult());

        protected override Task HandleChallengeAsync(AuthenticationProperties properties)
        {
            Response.StatusCode = StatusCodes.Status401Unauthorized;
            return Task.CompletedTask;
        }
    }
}
