using System;
using System.Globalization;
using Volo.Abp;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// The refusals of the AI pipeline, each with the <c>error.data</c> the web app reads (spec 5.8) and the feature id.
/// None of them ever carries provider text.
/// </summary>
public static class AiErrors
{
    /// <summary><see cref="RateLimited"/> reasons.</summary>
    public const string MinuteReason = "minute";
    public const string BusyReason = "busy";

    /// <summary><see cref="QuotaExceeded"/> scopes.</summary>
    public const string UserDayScope = "user-day";
    public const string WorkspaceMonthScope = "workspace-month";
    public const string PlatformMonthScope = "platform-month";

    /// <summary>The <c>data.kind</c> of <see cref="ProviderUnavailable"/> while the circuit breaker is open.</summary>
    public const string CircuitOpenKind = "circuit-open";

    /// <summary><c>Tracker:AiRateLimited</c> (429): the per-minute limit, or a request of the caller already in flight.</summary>
    public static BusinessException RateLimited(string reason, int retryAfterSeconds, string featureId) =>
        new BusinessException(TrackerDomainErrorCodes.AiRateLimited)
            .WithData("reason", reason)
            .WithData("retryAfterSeconds", Math.Max(1, retryAfterSeconds))
            .WithData("featureId", featureId);

    /// <summary><c>Tracker:AiQuotaExceeded</c> (429): an allowance is used up until <paramref name="resetsAt"/>.</summary>
    public static BusinessException QuotaExceeded(string scope, DateTime resetsAt, string featureId) =>
        new BusinessException(TrackerDomainErrorCodes.AiQuotaExceeded)
            .WithData("scope", scope)
            .WithData("resetsAt", FormatUtc(resetsAt))
            .WithData("featureId", featureId);

    /// <summary><c>Tracker:AiProviderUnavailable</c> (503) for a feature without a fallback.</summary>
    /// <param name="kind">timeout, rate-limited, error or circuit-open.</param>
    public static BusinessException ProviderUnavailable(string kind, int? retryAfterSeconds, string featureId)
    {
        var error = new BusinessException(TrackerDomainErrorCodes.AiProviderUnavailable)
            .WithData("kind", kind)
            .WithData("featureId", featureId);
        return retryAfterSeconds.HasValue ? error.WithData("retryAfterSeconds", retryAfterSeconds.Value) : error;
    }

    /// <summary><c>Tracker:AiOutputInvalid</c> (502): the answer stayed invalid after the repair turn and the feature has
    /// no fallback.</summary>
    public static BusinessException OutputInvalid(string featureId) =>
        new BusinessException(TrackerDomainErrorCodes.AiOutputInvalid).WithData("featureId", featureId);

    /// <summary>An instant as ISO 8601 UTC to the second ("2026-11-01T00:00:00Z").</summary>
    public static string FormatUtc(DateTime value)
    {
        var utc = value.Kind == DateTimeKind.Local ? value.ToUniversalTime() : DateTime.SpecifyKind(value, DateTimeKind.Utc);
        return utc.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
    }
}
