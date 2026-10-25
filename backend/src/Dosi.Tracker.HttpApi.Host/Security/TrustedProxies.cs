using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.Extensions.Configuration;

namespace Dosi.Tracker.Security;

/// <summary>
/// Which reverse proxies may tell the API the real client address and scheme (<c>X-Forwarded-For</c> /
/// <c>X-Forwarded-Proto</c>). The sign-in rate limits are kept per client IP, so behind a load balancer or ingress the
/// API must see the caller's address rather than the proxy's — but only a proxy the operator lists may say so: a header
/// from anyone else is ignored, otherwise every caller could pick a fresh "IP" per request and walk around the limits.
/// <para>Configuration (<c>App:ForwardedHeaders</c>): <c>KnownProxies</c> — proxy IP addresses; <c>KnownNetworks</c> —
/// proxy networks in CIDR notation (e.g. a Kubernetes pod network <c>10.0.0.0/8</c>); <c>ForwardLimit</c> — how many
/// proxy hops to unwind (default 1). Each list is a JSON array or a comma-separated string (environment variables:
/// <c>App__ForwardedHeaders__KnownNetworks=10.0.0.0/8</c>). Loopback stays trusted, so a proxy on the same host works
/// without configuration.</para>
/// </summary>
public static class TrustedProxies
{
    public const string SectionName = "App:ForwardedHeaders";

    /// <summary>Applies the configured proxies to <paramref name="options"/> (both forwarded headers are processed).
    /// Throws <see cref="InvalidOperationException"/> for an entry that is not an IP address / CIDR network, so a typo
    /// fails at start-up instead of silently trusting nothing.</summary>
    public static void Configure(ForwardedHeadersOptions options, IConfiguration configuration)
    {
        options.ForwardedHeaders = ForwardedHeaders.XForwardedFor | ForwardedHeaders.XForwardedProto;

        var section = configuration.GetSection(SectionName);

        foreach (var entry in ReadList(section.GetSection("KnownProxies")))
        {
            if (!IPAddress.TryParse(entry, out var address))
            {
                throw new InvalidOperationException($"{SectionName}:KnownProxies: '{entry}' is not an IP address.");
            }

            options.KnownProxies.Add(address);
        }

        foreach (var entry in ReadList(section.GetSection("KnownNetworks")))
        {
            if (!System.Net.IPNetwork.TryParse(entry, out var network))
            {
                throw new InvalidOperationException(
                    $"{SectionName}:KnownNetworks: '{entry}' is not a network in CIDR notation (e.g. 10.0.0.0/8).");
            }

            options.KnownIPNetworks.Add(network);
        }

        var forwardLimit = section.GetValue<int?>("ForwardLimit");
        if (forwardLimit.HasValue)
        {
            if (forwardLimit.Value < 1)
            {
                throw new InvalidOperationException($"{SectionName}:ForwardLimit must be at least 1.");
            }

            options.ForwardLimit = forwardLimit.Value;
        }
    }

    /// <summary>True when any proxy or proxy network is configured. Without one, only a proxy on the same host is
    /// trusted: behind a containerised proxy or an ingress the API then sees plain HTTP from the proxy's address — OpenIddict
    /// refuses the requests when HTTPS is required, and every caller shares the proxy's rate-limit budget.</summary>
    public static bool HasConfiguredProxies(IConfiguration configuration)
    {
        var section = configuration.GetSection(SectionName);
        return ReadList(section.GetSection("KnownProxies")).Any() || ReadList(section.GetSection("KnownNetworks")).Any();
    }

    /// <summary>A configured list: a JSON array (or indexed environment variables) or one comma/semicolon-separated string.</summary>
    private static IEnumerable<string> ReadList(IConfigurationSection section)
    {
        if (!string.IsNullOrWhiteSpace(section.Value))
        {
            return section.Value.Split(new[] { ',', ';' }, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        }

        return section.GetChildren()
            .Select(child => child.Value?.Trim())
            .Where(value => !string.IsNullOrEmpty(value))
            .Select(value => value!)
            .ToList();
    }
}
