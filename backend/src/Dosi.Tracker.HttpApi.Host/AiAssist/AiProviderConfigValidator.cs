using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.AiAssist.Provider;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// Decides from the <c>AI</c> configuration whether this server may call its AI provider. Every problem turns AI assist
/// off with a state naming it, never a failed start-up: the rest of the tracker works without AI. Pure (no host
/// dependencies), so it is unit-tested directly.
/// </summary>
public static class AiProviderConfigValidator
{
    public const int MinTimeoutSeconds = 1;
    public const int MaxTimeoutSeconds = 300;
    public const int MaxRetries = 5;

    private static readonly string[] SupportedProviders =
    {
        AiOptions.ProviderOpenAi, AiOptions.ProviderAzureOpenAi, AiOptions.ProviderOpenAiCompatible
    };

    /// <summary>
    /// In order: off unless <c>AI:Enabled</c>; a known <c>AI:Provider</c>; a key; a default model; then the endpoint.
    /// <c>AI:OpenAI:BaseUrl</c> is required for Azure OpenAI and compatible servers. When set it must be absolute https
    /// (plain http only for localhost or 127.0.0.1, and only in Development), carry no credentials, query or fragment,
    /// and its host must be listed in <c>AI:OpenAI:AllowedHosts</c>.
    /// </summary>
    public static AiProviderState Evaluate(AiOptions options, bool apiKeyPresent, bool isDevelopment)
    {
        if (!options.Enabled)
        {
            return AiProviderState.Disabled;
        }

        var provider = NormalizeProvider(options.Provider);
        if (provider == null)
        {
            return AiProviderState.UnsupportedProvider;
        }

        if (!apiKeyPresent)
        {
            return AiProviderState.MissingApiKey;
        }

        if (string.IsNullOrWhiteSpace(options.OpenAI.Model))
        {
            return AiProviderState.MissingModel;
        }

        var baseUrl = options.OpenAI.BaseUrl?.Trim();
        if (string.IsNullOrEmpty(baseUrl))
        {
            return provider == AiOptions.ProviderOpenAi ? AiProviderState.Configured : AiProviderState.InvalidEndpoint;
        }

        return IsAllowedEndpoint(baseUrl, ParseAllowedHosts(options.OpenAI.AllowedHosts), isDevelopment)
            ? AiProviderState.Configured
            : AiProviderState.InvalidEndpoint;
    }

    /// <summary>The supported provider name in its canonical casing, or null.</summary>
    public static string? NormalizeProvider(string? provider)
    {
        var trimmed = provider?.Trim();
        return SupportedProviders.FirstOrDefault(p => string.Equals(p, trimmed, StringComparison.OrdinalIgnoreCase));
    }

    /// <summary>The configured endpoint, or null for OpenAI's own. Only meaningful once <see cref="Evaluate"/> said
    /// Configured.</summary>
    public static Uri? GetEndpoint(AiOptions options)
    {
        var baseUrl = options.OpenAI.BaseUrl?.Trim();
        return string.IsNullOrEmpty(baseUrl) ? null : new Uri(baseUrl, UriKind.Absolute);
    }

    public static IReadOnlyCollection<string> ParseAllowedHosts(string? allowedHosts)
    {
        return (allowedHosts ?? "")
            .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Select(h => h.TrimEnd('.').ToLowerInvariant())
            .Where(h => h.Length > 0)
            .Distinct()
            .ToArray();
    }

    public static bool IsAllowedEndpoint(string baseUrl, IReadOnlyCollection<string> allowedHosts, bool isDevelopment)
    {
        if (!Uri.TryCreate(baseUrl, UriKind.Absolute, out var uri) ||
            !string.IsNullOrEmpty(uri.UserInfo) ||
            !string.IsNullOrEmpty(uri.Query) ||
            !string.IsNullOrEmpty(uri.Fragment))
        {
            return false;
        }

        var host = uri.IdnHost.TrimEnd('.').ToLowerInvariant();
        var schemeAllowed = uri.Scheme == Uri.UriSchemeHttps ||
                            (uri.Scheme == Uri.UriSchemeHttp && isDevelopment && host is "localhost" or "127.0.0.1");

        return schemeAllowed && allowedHosts.Contains(host);
    }

    /// <summary>The per-attempt timeout: <c>AI:RequestTimeoutSeconds</c> within 1-300 s.</summary>
    public static TimeSpan GetNetworkTimeout(AiOptions options) =>
        TimeSpan.FromSeconds(Math.Clamp(options.RequestTimeoutSeconds, MinTimeoutSeconds, MaxTimeoutSeconds));

    /// <summary><c>AI:MaxRetries</c> within 0-5.</summary>
    public static int GetMaxRetries(AiOptions options) => Math.Clamp(options.MaxRetries, 0, MaxRetries);

    /// <summary>What may be said about a configured provider; never its key or endpoint.</summary>
    public static AiProviderDescriptor Describe(AiOptions options)
    {
        var provider = NormalizeProvider(options.Provider) ?? AiOptions.ProviderOpenAi;
        var displayName = string.IsNullOrWhiteSpace(options.Disclosure.ProviderDisplayName)
            ? provider
            : options.Disclosure.ProviderDisplayName.Trim();

        return new AiProviderDescriptor(
            displayName,
            provider,
            options.OpenAI.Model.Trim(),
            SupportsJsonSchema: !string.Equals(options.StructuredOutput?.Trim(), AiOptions.StructuredOutputJsonObject,
                StringComparison.OrdinalIgnoreCase),
            SupportsTools: options.SupportsTools);
    }
}
