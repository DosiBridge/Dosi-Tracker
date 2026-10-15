using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.TestHost;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// The sign-in rate limits, run through the same middleware order as the API host: forwarded headers, routing, CORS,
/// then the limiter. Limits are configured low so the tests stay fast.
/// </summary>
public sealed class AuthRateLimitingTests : IAsyncLifetime
{
    private const string WebOrigin = "http://localhost:6005";
    private const string TrustedProxy = "10.0.0.5";

    private IHost _host = null!;
    private TestServer _server = null!;

    public async Task InitializeAsync()
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["App:RateLimiting:TokenPermitLimit"] = "6",
                ["App:RateLimiting:TokenPerAccountPermitLimit"] = "3",
                ["App:RateLimiting:RefreshPermitLimit"] = "8",
                ["App:RateLimiting:AccountPermitLimit"] = "2",
                ["App:RateLimiting:SignUpPermitLimit"] = "2",
                ["App:ForwardedHeaders:KnownProxies"] = TrustedProxy
            })
            .Build();

        _host = new HostBuilder()
            .ConfigureWebHost(web => web
                .UseTestServer()
                .ConfigureServices(services =>
                {
                    services.AddRouting();
                    services.AddCors(options => options.AddDefaultPolicy(policy => policy
                        .WithOrigins(WebOrigin)
                        .AllowAnyHeader()
                        .AllowAnyMethod()
                        .AllowCredentials()));
                    services.Configure<ForwardedHeadersOptions>(options => TrustedProxies.Configure(options, configuration));
                    services.AddTrackerAuthRateLimiting(configuration);
                })
                .Configure(app =>
                {
                    app.UseForwardedHeaders();
                    app.UseRouting();
                    app.UseCors();
                    app.UseTrackerAuthRateLimiting();
                    app.UseEndpoints(endpoints =>
                    {
                        // Stands in for OpenIddict, which reads the (already parsed) form again.
                        endpoints.MapPost("/connect/token", async context =>
                        {
                            var form = await context.Request.ReadFormAsync();
                            await context.Response.WriteAsync("issued:" + form["grant_type"]);
                        });
                        endpoints.MapPost("/api/account/login", () => "signed in");
                        endpoints.MapPost("/api/app/workspace/register", () => "registered");
                        endpoints.MapPost("/api/app/activity", () => "stored");
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

    private Task<HttpContext> PostAsync(
        string path,
        string? form = null,
        string peer = "203.0.113.10",
        string? forwardedFor = null,
        string? origin = null,
        string method = "POST")
    {
        return _server.SendAsync(context =>
        {
            context.Request.Method = method;
            context.Request.Path = path;
            context.Connection.RemoteIpAddress = IPAddress.Parse(peer);
            if (forwardedFor != null)
            {
                context.Request.Headers["X-Forwarded-For"] = forwardedFor;
            }

            if (origin != null)
            {
                context.Request.Headers.Origin = origin;
            }

            var body = Encoding.UTF8.GetBytes(form ?? "{}");
            context.Request.ContentType = form != null ? "application/x-www-form-urlencoded" : "application/json";
            context.Request.ContentLength = body.Length;
            context.Request.Body = new MemoryStream(body);
        });
    }

    private Task<HttpContext> PasswordGrantAsync(string userName, string peer = "203.0.113.10", string? forwardedFor = null, string? origin = null)
    {
        return PostAsync("/connect/token", $"grant_type=password&client_id=Tracker_App&username={userName}&password=wrong",
            peer, forwardedFor, origin);
    }

    private static Task<string> BodyOfAsync(HttpContext context)
    {
        return new StreamReader(context.Response.Body).ReadToEndAsync();
    }

    [Fact]
    public async Task Guessing_One_Accounts_Password_Should_Be_Stopped_With_A_Retry_After()
    {
        for (var attempt = 0; attempt < 3; attempt++)
        {
            (await PasswordGrantAsync("alice")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        }

        var rejected = await PasswordGrantAsync("ALICE"); // the account name is case-insensitive

        rejected.Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
        int.Parse(rejected.Response.Headers.RetryAfter.ToString()).ShouldBeInRange(1, 60);
        using var body = JsonDocument.Parse(await BodyOfAsync(rejected));
        body.RootElement.GetProperty("error").GetString().ShouldBe("too_many_requests");
        body.RootElement.GetProperty("error_description").GetString()!.ShouldContain("Try again");

        // Another account from the same address is still served (the refused attempt did use up some of the
        // address's budget, so hammering one account also slows the attacker down overall).
        (await PasswordGrantAsync("bob")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
    }

    [Fact]
    public async Task One_Address_Spraying_Many_Accounts_Should_Hit_The_Per_Client_Limit()
    {
        for (var account = 0; account < 6; account++)
        {
            (await PasswordGrantAsync($"user{account}")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        }

        (await PasswordGrantAsync("user6")).Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);

        // Other callers are not affected.
        (await PasswordGrantAsync("user6", peer: "203.0.113.99")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
    }

    [Fact]
    public async Task Refreshing_A_Session_Should_Have_Its_Own_More_Generous_Budget()
    {
        for (var attempt = 0; attempt < 6; attempt++)
        {
            await PasswordGrantAsync($"user{attempt}");
        }

        (await PasswordGrantAsync("someone")).Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);

        for (var refresh = 0; refresh < 8; refresh++)
        {
            var renewed = await PostAsync("/connect/token", "grant_type=refresh_token&client_id=Tracker_App&refresh_token=abc");
            renewed.Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
            (await BodyOfAsync(renewed)).ShouldBe("issued:refresh_token"); // the endpoint still reads the form
        }

        (await PostAsync("/connect/token", "grant_type=refresh_token&refresh_token=abc"))
            .Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
    }

    [Fact]
    public async Task Behind_The_Trusted_Proxy_Every_Client_Should_Get_Its_Own_Budget()
    {
        for (var attempt = 0; attempt < 6; attempt++)
        {
            (await PasswordGrantAsync($"office{attempt}", peer: TrustedProxy, forwardedFor: "198.51.100.1"))
                .Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        }

        (await PasswordGrantAsync("office6", peer: TrustedProxy, forwardedFor: "198.51.100.1"))
            .Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
        (await PasswordGrantAsync("office6", peer: TrustedProxy, forwardedFor: "198.51.100.2"))
            .Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
    }

    [Fact]
    public async Task A_Forwarded_Address_From_Anyone_Else_Should_Be_Ignored()
    {
        // An attacker inventing a new X-Forwarded-For per request still shares one budget: their own address.
        for (var attempt = 0; attempt < 6; attempt++)
        {
            (await PasswordGrantAsync($"victim{attempt}", peer: "192.0.2.9", forwardedFor: $"198.51.100.{attempt + 10}"))
                .Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        }

        (await PasswordGrantAsync("victim6", peer: "192.0.2.9", forwardedFor: "198.51.100.200"))
            .Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
    }

    [Fact]
    public async Task A_Rejection_Should_Carry_The_Cors_Headers_So_The_Web_App_Can_Read_It()
    {
        for (var attempt = 0; attempt < 3; attempt++)
        {
            await PasswordGrantAsync("alice", origin: WebOrigin);
        }

        var rejected = await PasswordGrantAsync("alice", origin: WebOrigin);

        rejected.Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
        rejected.Response.Headers.AccessControlAllowOrigin.ToString().ShouldBe(WebOrigin);
    }

    [Fact]
    public async Task Account_Endpoints_And_Sign_Up_Should_Be_Limited_Per_Client()
    {
        (await PostAsync("/api/account/login")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        (await PostAsync("/api/account/login")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        var rejected = await PostAsync("/api/account/login");
        rejected.Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
        using (var body = JsonDocument.Parse(await BodyOfAsync(rejected)))
        {
            // ABP clients read error.message.
            body.RootElement.GetProperty("error").GetProperty("message").GetString()!.ShouldContain("Too many attempts");
        }

        (await PostAsync("/api/app/workspace/register")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        (await PostAsync("/api/app/workspace/register")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        (await PostAsync("/api/app/workspace/register")).Response.StatusCode.ShouldBe(StatusCodes.Status429TooManyRequests);
    }

    [Fact]
    public async Task Everything_Else_Should_Never_Be_Limited()
    {
        for (var upload = 0; upload < 20; upload++)
        {
            (await PostAsync("/api/app/activity")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
        }

        // CORS preflights of the token endpoint are answered by CORS and never counted.
        for (var preflight = 0; preflight < 10; preflight++)
        {
            var response = await _server.SendAsync(context =>
            {
                context.Request.Method = HttpMethods.Options;
                context.Request.Path = "/connect/token";
                context.Connection.RemoteIpAddress = IPAddress.Parse("203.0.113.10");
                context.Request.Headers.Origin = WebOrigin;
                context.Request.Headers.AccessControlRequestMethod = "POST";
            });
            response.Response.StatusCode.ShouldBe(StatusCodes.Status204NoContent);
        }

        (await PasswordGrantAsync("alice")).Response.StatusCode.ShouldBe(StatusCodes.Status200OK);
    }

    [Theory]
    [InlineData("POST", "/connect/token", "grant_type=password", AuthRateLimiting.RequestKind.TokenGrant)]
    [InlineData("POST", "/connect/token", "grant_type=refresh_token", AuthRateLimiting.RequestKind.RefreshGrant)]
    [InlineData("POST", "/connect/token", "grant_type=authorization_code", AuthRateLimiting.RequestKind.TokenGrant)]
    [InlineData("POST", "/Account/Login", null, AuthRateLimiting.RequestKind.Account)]
    [InlineData("POST", "/api/account/my-profile/change-password", null, AuthRateLimiting.RequestKind.Account)]
    [InlineData("POST", "/api/app/workspace/register", null, AuthRateLimiting.RequestKind.SignUp)]
    [InlineData("POST", "/api/app/workspaces", null, AuthRateLimiting.RequestKind.SignUp)]
    [InlineData("POST", "/api/app/workspaces/", null, AuthRateLimiting.RequestKind.SignUp)]
    [InlineData("POST", "/api/app/workspaces/0f8fad5b-d9cb-469f-a165-70867728950e/leave", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("GET", "/api/app/workspaces/mine", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("POST", "/api/app/invitations/abc123/accept", null, AuthRateLimiting.RequestKind.Account)]
    [InlineData("GET", "/api/app/invitations/abc123", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("POST", "/api/app/workspace/find-my-workspaces", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("GET", "/connect/token", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("GET", "/api/account/my-profile", null, AuthRateLimiting.RequestKind.None)]
    [InlineData("POST", "/api/app/team/invite-member", null, AuthRateLimiting.RequestKind.None)]
    public void Requests_Should_Be_Classified_By_Endpoint_And_Grant(string method, string path, string? form, AuthRateLimiting.RequestKind expected)
    {
        var context = new DefaultHttpContext();
        context.Request.Method = method;
        context.Request.Path = path;
        if (form != null)
        {
            // What the pipeline step before the limiter stores for a token request.
            context.Items[AuthRateLimiting.GrantTypeItem] = form.Split('=')[1];
        }

        AuthRateLimiting.Classify(context).ShouldBe(expected);
    }
}
