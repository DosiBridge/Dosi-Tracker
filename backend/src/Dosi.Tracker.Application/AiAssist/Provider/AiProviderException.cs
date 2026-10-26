using System;

namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>How a provider call failed.</summary>
public enum AiProviderErrorKind
{
    /// <summary>No answer in time (408, or the per-attempt timeout).</summary>
    Timeout,

    /// <summary>429: the provider's rate limit or the account's quota.</summary>
    RateLimited,

    /// <summary>401/403: the key is wrong, revoked or not allowed this model.</summary>
    Auth,

    /// <summary>Another 4xx: the request itself was refused (for example an unknown model or parameter).</summary>
    BadRequest,

    /// <summary>A 5xx (including Cloudflare's 52x) or an unexpected status.</summary>
    ServerError,

    /// <summary>The provider could not be reached.</summary>
    Network,

    /// <summary>The provider's content filter refused the request or the answer.</summary>
    ContentFilter
}

/// <summary>
/// A failed provider call, after the provider SDK's own retries. It deliberately carries no provider text and no inner
/// exception: provider messages can echo request content, account details or key fragments, and nothing of them may
/// reach a DTO, a log or the audit log.
/// </summary>
public sealed class AiProviderException : Exception
{
    public AiProviderException(AiProviderErrorKind kind, int? status = null, int? retryAfterSeconds = null)
        : base(BuildMessage(kind, status))
    {
        Kind = kind;
        Status = status;
        RetryAfterSeconds = retryAfterSeconds;
    }

    public AiProviderErrorKind Kind { get; }

    /// <summary>The provider's HTTP status, when it answered.</summary>
    public int? Status { get; }

    /// <summary>The provider's Retry-After, in seconds, when it sent one.</summary>
    public int? RetryAfterSeconds { get; }

    /// <summary>A failure worth retrying later (and counted by the circuit breaker): timeouts, rate limits, server and
    /// network errors. Auth, bad requests and content filtering will fail the same way again.</summary>
    public bool IsTransient => AiProviderErrorMapper.IsTransient(Kind);

    private static string BuildMessage(AiProviderErrorKind kind, int? status) =>
        status.HasValue ? $"The AI provider call failed: {kind} (HTTP {status.Value})." : $"The AI provider call failed: {kind}.";
}
