using System;
using System.Linq;
using Microsoft.Extensions.Configuration;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// Absolute links into the web app, for emails. The web app's address is <see cref="ClientUrlKey"/>
/// (<c>App:ClientUrl</c>, e.g. <c>https://app.example.com</c>) or, when that is not set, the first of
/// <c>App:CorsOrigins</c> (the web app is the origin the API admits). The pages the links open:
/// <list type="bullet">
/// <item><c>/invite/{token}</c> — an invitation;</item>
/// <item><c>/reset-password?userId=&amp;resetToken=[&amp;returnUrl=]</c> — completes ABP's
/// <c>POST /api/account/reset-password</c>;</item>
/// <item><c>/confirm-email?userId=&amp;token=</c> — completes <c>POST /api/app/my-account/confirm-email</c>;</item>
/// <item><c>/billing</c> — the workspace's billing page.</item>
/// </list>
/// Every link is null when no web app address is known; the caller then sends no email.
/// </summary>
public class WebAppLinks : ITransientDependency
{
    public const string ClientUrlKey = "App:ClientUrl";
    public const string CorsOriginsKey = "App:CorsOrigins";

    public WebAppLinks(IConfiguration configuration)
    {
        BaseUrl = ResolveBaseUrl(configuration);
    }

    /// <summary>The web app's address without a trailing slash, or null when none is configured.</summary>
    public string? BaseUrl { get; }

    public string? Invitation(string token) => Build("invite/" + Uri.EscapeDataString(token));

    public string? ResetPassword(Guid userId, string resetToken, string? returnUrl = null)
    {
        var query = $"reset-password?userId={userId:D}&resetToken={Uri.EscapeDataString(resetToken)}";
        var safeReturnUrl = SafeReturnUrl(returnUrl);
        return Build(safeReturnUrl == null ? query : query + "&returnUrl=" + Uri.EscapeDataString(safeReturnUrl));
    }

    public string? ConfirmEmail(Guid userId, string token) =>
        Build($"confirm-email?userId={userId:D}&token={Uri.EscapeDataString(token)}");

    public string? Billing() => Build("billing");

    /// <summary>The configured web app address: <c>App:ClientUrl</c>, else the first <c>App:CorsOrigins</c> entry, as an
    /// absolute http(s) URL without a trailing slash; null when neither is a usable address.</summary>
    public static string? ResolveBaseUrl(IConfiguration configuration)
    {
        var candidate = configuration[ClientUrlKey];
        if (string.IsNullOrWhiteSpace(candidate))
        {
            candidate = configuration[CorsOriginsKey]?
                .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .FirstOrDefault(origin => !origin.Contains('*'));
        }

        if (string.IsNullOrWhiteSpace(candidate) ||
            !Uri.TryCreate(candidate.Trim(), UriKind.Absolute, out var uri) ||
            (uri.Scheme != Uri.UriSchemeHttps && uri.Scheme != Uri.UriSchemeHttp))
        {
            return null;
        }

        return uri.GetLeftPart(UriPartial.Path).TrimEnd('/');
    }

    /// <summary>A return URL the web app may follow after the reset: a path on the web app itself (<c>/team</c>), never
    /// another site (<c>//evil.example</c>, <c>https://…</c>, <c>/\evil</c>). Anything else is dropped.</summary>
    public static string? SafeReturnUrl(string? returnUrl)
    {
        if (string.IsNullOrWhiteSpace(returnUrl))
        {
            return null;
        }

        var trimmed = returnUrl.Trim();
        var isLocalPath = trimmed.StartsWith('/') &&
                          !trimmed.StartsWith("//", StringComparison.Ordinal) &&
                          !trimmed.StartsWith("/\\", StringComparison.Ordinal) &&
                          !trimmed.Any(char.IsControl);
        return isLocalPath ? trimmed : null;
    }

    private string? Build(string pathAndQuery) => BaseUrl == null ? null : BaseUrl + "/" + pathAndQuery;
}
