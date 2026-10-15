using System;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.RateLimiting;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.RateLimiting;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker.Security;

/// <summary>
/// Brute-force protection for the anonymous entry points: the OpenIddict token endpoint, ABP's account endpoints
/// (cookie login, password checks and resets), invitation acceptance, sign-up and workspace creation. Every limit is per real client IP —
/// which requires <see cref="TrustedProxies"/> behind a proxy — so one abusive address cannot block everyone else:
/// <list type="bullet">
/// <item>password (and other non-refresh) token grants: <see cref="AuthRateLimitOptions.TokenPermitLimit"/> per
/// client — an office behind one NAT signs in together at 9:00 — and
/// <see cref="AuthRateLimitOptions.TokenPerAccountPermitLimit"/> per client and user name (or email), which stops
/// guessing one account's password;</item>
/// <item>refresh-token grants: <see cref="AuthRateLimitOptions.RefreshPermitLimit"/> per client (agents renew their
/// sessions this way, and a refresh token cannot be guessed);</item>
/// <item>account endpoints (and invitation acceptance), and sign-up / workspace creation: <see cref="AuthRateLimitOptions.AccountPermitLimit"/> /
/// <see cref="AuthRateLimitOptions.SignUpPermitLimit"/> per client.</item>
/// </list>
/// A request refused by the per-account limit still uses up permits of its client's budget, so hammering one account
/// also slows the attacker down overall. A rejection is a 429 with <c>Retry-After</c>. The limiter must run after the
/// CORS middleware so that a browser can read that 429 (otherwise the web app only sees a failed request and reports the
/// server as unreachable).
/// </summary>
public static class AuthRateLimiting
{
    public static readonly PathString TokenPath = new("/connect/token");
    public static readonly PathString WorkspaceRegisterPath = new("/api/app/workspace/register");
    public static readonly PathString WorkspacesPath = new("/api/app/workspaces");
    public static readonly PathString InvitationsPath = new("/api/app/invitations");

    /// <summary>Endpoints that take a password, issue a session or start a password reset (ABP account API and pages).</summary>
    public static readonly PathString[] AccountPaths =
    {
        new("/api/account/login"),
        new("/api/account/check-password"),
        new("/api/account/register"),
        new("/api/account/send-password-reset-code"),
        new("/api/account/verify-password-reset-token"),
        new("/api/account/reset-password"),
        new("/api/account/my-profile/change-password"),
        new("/Account/Login"),
        new("/Account/Register"),
        new("/Account/ForgotPassword"),
        new("/Account/ResetPassword")
    };

    internal const string GrantTypeItem = "Dosi.Tracker.AuthRateLimiting.GrantType";
    private const string AccountItem = "Dosi.Tracker.AuthRateLimiting.Account";
    private const string RefreshTokenGrantType = "refresh_token";

    /// <summary>The kind of request, which decides its limits.</summary>
    public enum RequestKind
    {
        /// <summary>Not rate-limited.</summary>
        None,

        /// <summary>A token request that is not a refresh (password, authorization code, …).</summary>
        TokenGrant,

        /// <summary>A refresh-token request.</summary>
        RefreshGrant,

        /// <summary>An account endpoint (login, password check/reset, registration).</summary>
        Account,

        /// <summary>Sign-up and workspace creation.</summary>
        SignUp
    }

    /// <summary>Registers the rate limiter with the limits of <see cref="AuthRateLimitOptions.SectionName"/>.</summary>
    public static IServiceCollection AddTrackerAuthRateLimiting(this IServiceCollection services, IConfiguration configuration)
    {
        var limits = AuthRateLimitOptions.From(configuration);

        services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            options.OnRejected = (context, cancellationToken) => WriteRejectionAsync(context, limits, cancellationToken);
            options.GlobalLimiter = PartitionedRateLimiter.CreateChained(
                PartitionedRateLimiter.Create<HttpContext, string>(context => ClientPartition(context, limits)),
                PartitionedRateLimiter.Create<HttpContext, string>(context => AccountPartition(context, limits)));
        });

        return services;
    }

    /// <summary>
    /// Adds the rate limiter to the pipeline, preceded by a step that reads the grant type and the account name of token
    /// requests (they are form fields; the parsed form is cached, so OpenIddict reads it again for free). Call it after
    /// <c>UseForwardedHeaders</c> (real client IP) and <c>UseCors</c> (a 429 must carry the CORS headers).
    /// </summary>
    public static IApplicationBuilder UseTrackerAuthRateLimiting(this IApplicationBuilder app)
    {
        app.Use(CaptureTokenRequestAsync);
        return app.UseRateLimiter();
    }

    /// <summary>Classifies a request. Only POSTs are limited (CORS preflights and pages are never counted).</summary>
    public static RequestKind Classify(HttpContext context)
    {
        var request = context.Request;
        if (!HttpMethods.IsPost(request.Method))
        {
            return RequestKind.None;
        }

        if (request.Path.StartsWithSegments(TokenPath))
        {
            return string.Equals(context.Items[GrantTypeItem] as string, RefreshTokenGrantType, StringComparison.Ordinal)
                ? RequestKind.RefreshGrant
                : RequestKind.TokenGrant;
        }

        if (request.Path.StartsWithSegments(WorkspaceRegisterPath, StringComparison.OrdinalIgnoreCase) ||
            IsWorkspaceCreation(request.Path))
        {
            return RequestKind.SignUp;
        }

        if (IsInvitationAcceptance(request.Path))
        {
            return RequestKind.Account;
        }

        return AccountPaths.Any(path => request.Path.StartsWithSegments(path, StringComparison.OrdinalIgnoreCase))
            ? RequestKind.Account
            : RequestKind.None;
    }

    /// <summary><c>POST /api/app/workspaces</c> itself (not its sub-routes such as <c>/{id}/leave</c>).</summary>
    private static bool IsWorkspaceCreation(PathString path)
    {
        return path.StartsWithSegments(WorkspacesPath, StringComparison.OrdinalIgnoreCase, out var rest) &&
               (!rest.HasValue || rest.Value == "/");
    }

    /// <summary><c>POST /api/app/invitations/{token}/accept</c>.</summary>
    private static bool IsInvitationAcceptance(PathString path)
    {
        return path.StartsWithSegments(InvitationsPath, StringComparison.OrdinalIgnoreCase, out var rest) &&
               rest.HasValue &&
               rest.Value!.TrimEnd('/').EndsWith("/accept", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>The caller's address as the API sees it (the forwarded one when a trusted proxy supplied it).</summary>
    public static string ClientAddress(HttpContext context)
    {
        return context.Connection.RemoteIpAddress?.ToString() ?? "unknown";
    }

    private static RateLimitPartition<string> ClientPartition(HttpContext context, AuthRateLimitOptions limits)
    {
        var kind = Classify(context);
        var permitLimit = kind switch
        {
            RequestKind.TokenGrant => limits.TokenPermitLimit,
            RequestKind.RefreshGrant => limits.RefreshPermitLimit,
            RequestKind.Account => limits.AccountPermitLimit,
            RequestKind.SignUp => limits.SignUpPermitLimit,
            _ => 0
        };

        if (permitLimit <= 0)
        {
            return RateLimitPartition.GetNoLimiter("unlimited");
        }

        return RateLimitPartition.GetFixedWindowLimiter(
            $"{kind}:{ClientAddress(context)}",
            _ => FixedWindow(permitLimit, limits.Window));
    }

    private static RateLimitPartition<string> AccountPartition(HttpContext context, AuthRateLimitOptions limits)
    {
        var account = context.Items[AccountItem] as string;
        if (Classify(context) != RequestKind.TokenGrant || string.IsNullOrEmpty(account) || limits.TokenPerAccountPermitLimit <= 0)
        {
            return RateLimitPartition.GetNoLimiter("unlimited");
        }

        return RateLimitPartition.GetFixedWindowLimiter(
            $"{RequestKind.TokenGrant}:{ClientAddress(context)}:{account}",
            _ => FixedWindow(limits.TokenPerAccountPermitLimit, limits.Window));
    }

    private static FixedWindowRateLimiterOptions FixedWindow(int permitLimit, TimeSpan window)
    {
        return new FixedWindowRateLimiterOptions
        {
            PermitLimit = permitLimit,
            Window = window,
            QueueLimit = 0
        };
    }

    /// <summary>Reads <c>grant_type</c> and the (case-insensitive) user name of a token request into the request items.</summary>
    private static async Task CaptureTokenRequestAsync(HttpContext context, Func<Task> next)
    {
        var request = context.Request;
        if (HttpMethods.IsPost(request.Method) && request.Path.StartsWithSegments(TokenPath) && request.HasFormContentType)
        {
            try
            {
                var form = await request.ReadFormAsync(context.RequestAborted);
                context.Items[GrantTypeItem] = form["grant_type"].ToString();

                var userName = form["username"].ToString().Trim();
                if (userName.Length > 0)
                {
                    context.Items[AccountItem] = userName.ToLowerInvariant();
                }
            }
            catch (InvalidDataException)
            {
                // A malformed form: limited as a (non-refresh) token request; OpenIddict rejects it itself.
            }
        }

        await next();
    }

    private static async ValueTask WriteRejectionAsync(
        OnRejectedContext context,
        AuthRateLimitOptions limits,
        CancellationToken cancellationToken)
    {
        var httpContext = context.HttpContext;
        var retryAfter = context.Lease.TryGetMetadata(MetadataName.RetryAfter, out var wait) && wait > TimeSpan.Zero
            ? wait
            : limits.Window;
        var seconds = Math.Max(1, (int)Math.Ceiling(retryAfter.TotalSeconds));

        var kind = Classify(httpContext);
        httpContext.RequestServices.GetService<ILoggerFactory>()?
            .CreateLogger(typeof(AuthRateLimiting).FullName!)
            .LogWarning("Rate limit hit: {Kind} request from {Client} to {Path}.", kind, ClientAddress(httpContext), httpContext.Request.Path);

        var message = $"Too many attempts. Try again in {seconds} seconds.";
        object body = kind is RequestKind.TokenGrant or RequestKind.RefreshGrant
            // OAuth clients (the web sign-in and the agents) read "error" / "error_description".
            ? new { error = "too_many_requests", error_description = message }
            // ABP clients read error.message.
            : new { error = new { message } };

        httpContext.Response.Headers.RetryAfter = seconds.ToString(CultureInfo.InvariantCulture);
        httpContext.Response.ContentType = "application/json";
        await httpContext.Response.WriteAsync(JsonSerializer.Serialize(body), cancellationToken);
    }
}

/// <summary>The sign-in rate limits (configuration section <see cref="SectionName"/>); 0 turns a limit off.</summary>
public class AuthRateLimitOptions
{
    public const string SectionName = "App:RateLimiting";

    /// <summary>Non-refresh token requests per client IP and window.</summary>
    public int TokenPermitLimit { get; set; } = 60;

    /// <summary>Non-refresh token requests per client IP and user name and window.</summary>
    public int TokenPerAccountPermitLimit { get; set; } = 10;

    /// <summary>Refresh-token requests per client IP and window.</summary>
    public int RefreshPermitLimit { get; set; } = 120;

    /// <summary>Account endpoint requests (login, password check/reset/change, registration) per client IP and window.</summary>
    public int AccountPermitLimit { get; set; } = 10;

    /// <summary>Sign-ups and workspace creations per client IP and window.</summary>
    public int SignUpPermitLimit { get; set; } = 10;

    /// <summary>Length of the fixed window, in seconds.</summary>
    public int WindowSeconds { get; set; } = 60;

    public TimeSpan Window => TimeSpan.FromSeconds(Math.Max(1, WindowSeconds));

    /// <summary>The defaults, overridden by whatever <see cref="SectionName"/> sets.</summary>
    public static AuthRateLimitOptions From(IConfiguration configuration)
    {
        var options = new AuthRateLimitOptions();
        configuration.GetSection(SectionName).Bind(options);
        return options;
    }
}
