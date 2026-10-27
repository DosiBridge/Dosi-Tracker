using System.Net;
using System.Text.Encodings.Web;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Diagnostics;
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

/// <summary>
/// ABP's <c>UseErrorPage()</c> (called from <see cref="TrackerHttpApiHostModule.OnApplicationInitialization"/>)
/// wires up <c>UseStatusCodePagesWithRedirects("/Error?httpStatusCode={0}")</c> — a genuine client-visible 302, not
/// a server-side re-execute — for any response whose body is still empty, true of a bearer challenge's 401, which
/// never writes one. Confirmed against a real Production deploy: <c>GET /api/app/workspaces/mine</c> with an
/// invalid access token answered an actual 302 with <c>Location: /Error?httpStatusCode=401</c> (and a
/// <c>WWW-Authenticate</c> header proving OpenIddict had already decided 401 correctly first), never reproduced
/// against a local/Development run because <c>UseErrorPage()</c> itself is skipped there. This reproduces that
/// exact shape — status-code-pages wired up the same way, same empty-bodied challenge — against a minimal host,
/// and proves <see cref="TrackerHttpApiHostModule"/>'s API opt-out (<see cref="IStatusCodePagesFeature.Enabled"/>
/// set to <see langword="false"/> for <c>/api</c> paths) keeps the real 401 instead of the redirect, while a
/// non-API path is unaffected (so the opt-out is scoped, not a global disable that would've silently made the
/// first assertion meaningless).
/// </summary>
public sealed class ApiStatusCodePagesTests : IAsyncLifetime
{
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
                    services.AddAuthentication("Bearer")
                        .AddScheme<AuthenticationSchemeOptions, EmptyBodyChallengeHandler>("Bearer", _ => { });
                })
                .Configure(app =>
                {
                    app.UseStatusCodePagesWithRedirects("/Error?httpStatusCode={0}");

                    // The same opt-out TrackerHttpApiHostModule.OnApplicationInitialization applies right after
                    // UseErrorPage(), reproduced verbatim (see that method for why the ordering works).
                    app.Use(async (httpContext, next) =>
                    {
                        if (httpContext.Request.Path.StartsWithSegments("/api"))
                        {
                            var statusCodePagesFeature = httpContext.Features.Get<IStatusCodePagesFeature>();
                            if (statusCodePagesFeature != null)
                            {
                                statusCodePagesFeature.Enabled = false;
                            }
                        }

                        await next();
                    });

                    app.UseRouting();
                    app.UseAuthentication();
                    app.UseAuthorization();
                    app.UseEndpoints(endpoints =>
                    {
                        endpoints.MapGet("/api/app/workspace/my-list", () => "ok").RequireAuthorization();
                        endpoints.MapGet("/not-api", () => "ok").RequireAuthorization();
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
    public async Task An_Api_Path_Keeps_The_Real_401_Instead_Of_The_Themed_Redirect()
    {
        using var client = _host.GetTestServer().CreateClient();

        var response = await client.GetAsync("/api/app/workspace/my-list");

        response.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        response.Headers.Location.ShouldBeNull();
    }

    [Fact]
    public async Task A_Non_Api_Path_Still_Gets_The_Themed_Redirect()
    {
        using var client = _host.GetTestServer().CreateClient();

        var response = await client.GetAsync("/not-api");

        // Proves the opt-out is scoped to /api, not a global disable that would make the test above meaningless.
        response.StatusCode.ShouldBe(HttpStatusCode.Redirect);
        response.Headers.Location!.OriginalString.ShouldBe("/Error?httpStatusCode=401");
    }

    /// <summary>Stands in for OpenIddict's bearer validation handler: challenges with a bare status code and no
    /// body, exactly what triggers ABP's status-code-pages redirect for an otherwise-correct 401.</summary>
    private sealed class EmptyBodyChallengeHandler : AuthenticationHandler<AuthenticationSchemeOptions>
    {
        public EmptyBodyChallengeHandler(
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
