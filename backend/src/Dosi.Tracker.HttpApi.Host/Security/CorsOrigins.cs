using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.Extensions.Configuration;

namespace Dosi.Tracker.Security;

/// <summary>
/// The browser origins the API admits (CORS), from <see cref="Key"/>: a comma-separated list such as
/// <c>https://app.example.com,https://*.example.com</c> (environment variable <c>App__CorsOrigins</c>). Each entry is an
/// absolute http(s) origin — scheme, host and optional port, no path — and may start its host with one wildcard label
/// (<c>https://*.example.com</c>); a trailing slash is dropped. A bare <c>*</c> is refused: the API admits credentialed
/// requests, so admitting every site would let any page act with a signed-in browser's session. A malformed entry stops
/// the start-up instead of silently admitting nothing.
/// </summary>
public static class CorsOrigins
{
    public const string Key = "App:CorsOrigins";

    public static string[] Parse(IConfiguration configuration) => Parse(configuration[Key]);

    public static string[] Parse(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return Array.Empty<string>();
        }

        var origins = new List<string>();
        foreach (var entry in value.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            var origin = entry.TrimEnd('/');
            if (!IsValidOrigin(origin))
            {
                throw new InvalidOperationException(
                    $"{Key}: '{entry}' is not an allowed origin. Use absolute origins such as https://app.example.com " +
                    "(optionally https://*.example.com); '*' is not allowed.");
            }

            if (!origins.Contains(origin, StringComparer.OrdinalIgnoreCase))
            {
                origins.Add(origin);
            }
        }

        return origins.ToArray();
    }

    private static bool IsValidOrigin(string origin)
    {
        var schemeEnd = origin.IndexOf("://", StringComparison.Ordinal);
        if (schemeEnd <= 0)
        {
            return false;
        }

        // One wildcard, as the whole first label of the host ("https://*.example.com").
        var candidate = origin;
        var wildcard = origin.IndexOf('*');
        if (wildcard >= 0)
        {
            if (wildcard != schemeEnd + 3 || origin.IndexOf('*', wildcard + 1) >= 0 ||
                origin.Length <= wildcard + 2 || origin[wildcard + 1] != '.')
            {
                return false;
            }

            candidate = origin.Remove(wildcard, 1).Insert(wildcard, "wildcard");
        }

        return Uri.TryCreate(candidate, UriKind.Absolute, out var uri) &&
               (uri.Scheme == Uri.UriSchemeHttps || uri.Scheme == Uri.UriSchemeHttp) &&
               string.IsNullOrEmpty(uri.UserInfo) &&
               uri.AbsolutePath == "/" &&
               string.IsNullOrEmpty(uri.Query) &&
               string.IsNullOrEmpty(uri.Fragment) &&
               !candidate.EndsWith("/", StringComparison.Ordinal) &&
               candidate.IndexOf('/', schemeEnd + 3) < 0;
    }
}
