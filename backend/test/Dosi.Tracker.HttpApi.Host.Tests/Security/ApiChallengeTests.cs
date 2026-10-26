using System.Net;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authentication.Cookies;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Identity;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// This host has no browser sign-in flow a human ever reaches (the product's login UI is the Next.js dashboard;
/// desktop agents are headless API clients), so a missing or rejected credential must answer 401/403, never the
/// ASP.NET Core Identity default of a 302 redirect to an Account/Login page that does not exist for API callers.
/// Reproduces <see cref="TrackerHttpApiHostModule"/>'s <c>ConfigureAuthentication</c> cookie-events override against
/// a minimal cookie-only pipeline (no OpenIddict/EF bootstrap needed to prove the redirect-to-login override).
/// </summary>
public sealed class ApiChallengeTests : IAsyncLifetime
{
    private IHost _host = null!;
    private TestServer _server = null!;

    public async Task InitializeAsync()
    {
        _host = new HostBuilder()
            .ConfigureWebHost(web => web
                .UseTestServer()
                .ConfigureServices(services =>
                {
                    services.AddRouting();
                    services.AddAuthorization();
                    services.AddAuthentication(IdentityConstants.ApplicationScheme)
                        .AddCookie(IdentityConstants.ApplicationScheme);

                    // The exact override TrackerHttpApiHostModule.ConfigureAuthentication applies.
                    services.Configure<CookieAuthenticationOptions>(IdentityConstants.ApplicationScheme, options =>
                    {
                        options.Events.OnRedirectToLogin = ctx =>
                        {
                            ctx.Response.StatusCode = StatusCodes.Status401Unauthorized;
                            return Task.CompletedTask;
                        };
                        options.Events.OnRedirectToAccessDenied = ctx =>
                        {
                            ctx.Response.StatusCode = StatusCodes.Status403Forbidden;
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
            .Build();

        await _host.StartAsync();
        _server = _host.GetTestServer();
    }

    public async Task DisposeAsync()
    {
        await _host.StopAsync();
        _host.Dispose();
    }

    [Fact]
    public async Task No_Credential_Answers_401_Not_A_Login_Redirect()
    {
        using var client = _server.CreateClient();

        var response = await client.GetAsync("/api/app/workspace/my-list");

        response.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
    }
}
