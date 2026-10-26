using System;
using System.Globalization;

namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>Pure classification of provider failures, shared by every provider implementation.</summary>
public static class AiProviderErrorMapper
{
    /// <summary>The longest Retry-After passed on to callers; anything longer is reported as this.</summary>
    public const int MaxRetryAfterSeconds = 3600;

    /// <summary>401/403 → Auth, 408 → Timeout, 429 → RateLimited, other 4xx → BadRequest, 5xx (and anything
    /// unexpected) → ServerError; no status (no answer at all) → Network.</summary>
    public static AiProviderErrorKind Map(int? status)
    {
        return status switch
        {
            null or <= 0 => AiProviderErrorKind.Network,
            401 or 403 => AiProviderErrorKind.Auth,
            408 => AiProviderErrorKind.Timeout,
            429 => AiProviderErrorKind.RateLimited,
            >= 400 and < 500 => AiProviderErrorKind.BadRequest,
            _ => AiProviderErrorKind.ServerError
        };
    }

    /// <summary>Whether a status is worth retrying at once (the provider SDK's retry policy): 408, 429 and every 5xx.
    /// OpenAI's edge answers 520-529 intermittently, which the SDK's default policy would not retry.</summary>
    public static bool IsRetriableStatus(int status) => status is 408 or 429 or (>= 500 and < 600);

    public static bool IsTransient(AiProviderErrorKind kind) =>
        kind is AiProviderErrorKind.Timeout or AiProviderErrorKind.RateLimited or
            AiProviderErrorKind.ServerError or AiProviderErrorKind.Network;

    /// <summary>The <c>data.kind</c> of <c>Tracker:AiProviderUnavailable</c>: timeout, rate-limited or error.</summary>
    public static string ToUnavailableKind(AiProviderErrorKind kind) => kind switch
    {
        AiProviderErrorKind.Timeout => "timeout",
        AiProviderErrorKind.RateLimited => "rate-limited",
        _ => "error"
    };

    /// <summary>
    /// The seconds a Retry-After header asks to wait: delta-seconds or an HTTP date (RFC 9110 §10.2.3), rounded up and
    /// capped at <see cref="MaxRetryAfterSeconds"/>; null when absent or unreadable. A date in the past is 0.
    /// </summary>
    public static int? ParseRetryAfterSeconds(string? value, DateTimeOffset now)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        value = value.Trim();
        if (double.TryParse(value, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out var seconds))
        {
            return Clamp(seconds);
        }

        if (DateTimeOffset.TryParseExact(value, "r", CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var date))
        {
            return Clamp((date - now).TotalSeconds);
        }

        return null;
    }

    private static int Clamp(double seconds)
    {
        if (double.IsNaN(seconds) || seconds <= 0)
        {
            return 0;
        }

        return seconds >= MaxRetryAfterSeconds ? MaxRetryAfterSeconds : (int)Math.Ceiling(seconds);
    }
}
