using System;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>Where <see cref="AiCircuitBreaker"/> stands.</summary>
public enum AiCircuitState
{
    /// <summary>Requests reach the provider.</summary>
    Closed,

    /// <summary>The provider is failing; requests are answered without calling it.</summary>
    Open,

    /// <summary>The pause is over; the next request probes the provider.</summary>
    HalfOpen
}

/// <summary>
/// Thrown by <see cref="AiCircuitBreaker.ThrowIfOpen"/>: the provider is not called now. The runner answers with the
/// feature's fallback, or <c>Tracker:AiProviderUnavailable</c> (kind circuit-open).
/// </summary>
public sealed class AiCircuitOpenException : Exception
{
    public AiCircuitOpenException(int retryAfterSeconds)
        : base("The AI provider is paused after repeated failures.")
    {
        RetryAfterSeconds = retryAfterSeconds;
    }

    public int RetryAfterSeconds { get; }
}

/// <summary>
/// Stops calling a provider that keeps failing, per API instance (a singleton):
/// <list type="bullet">
/// <item><see cref="FailureThreshold"/> transient failures in a row (timeouts, rate limits, server and network errors)
/// within <see cref="FailureWindow"/> open it for <see cref="OpenDuration"/>; then one request probes the provider
/// (half-open) and closes it on success or opens it again on failure.</item>
/// <item>A rejected key (401/403) opens it for <see cref="AuthOpenDuration"/> at once and logs a Critical line,
/// never the key or the provider's text.</item>
/// <item>Any answer, even a refused request (400, content filter), shows the provider is reachable.</item>
/// </list>
/// </summary>
public class AiCircuitBreaker
{
    public const int FailureThreshold = 5;
    public static readonly TimeSpan FailureWindow = TimeSpan.FromSeconds(60);
    public static readonly TimeSpan OpenDuration = TimeSpan.FromSeconds(60);
    public static readonly TimeSpan AuthOpenDuration = TimeSpan.FromMinutes(10);

    private readonly ILogger<AiCircuitBreaker> _logger;
    private readonly TimeProvider _time;
    private readonly object _sync = new();

    private int _failures;
    private DateTimeOffset _firstFailureAt;
    private DateTimeOffset? _openUntil;
    private bool _probeInFlight;

    public AiCircuitBreaker(ILogger<AiCircuitBreaker> logger, TimeProvider time)
    {
        _logger = logger;
        _time = time;
    }

    public AiCircuitState State
    {
        get
        {
            lock (_sync)
            {
                return CurrentState(_time.GetUtcNow());
            }
        }
    }

    /// <summary>The state on the platform overview: "closed", "open" or "half-open".</summary>
    public string StateName => ToWire(State);

    public static string ToWire(AiCircuitState state) => state switch
    {
        AiCircuitState.Open => "open",
        AiCircuitState.HalfOpen => "half-open",
        _ => "closed"
    };

    /// <summary>Lets the call through, or throws <see cref="AiCircuitOpenException"/> while open, and while half-open
    /// once another request is already probing. Every call let through must end in <see cref="RecordSuccess"/>,
    /// <see cref="RecordFailure"/> or <see cref="RecordAbandoned"/>.</summary>
    public virtual void ThrowIfOpen()
    {
        lock (_sync)
        {
            var now = _time.GetUtcNow();
            switch (CurrentState(now))
            {
                case AiCircuitState.Open:
                    throw new AiCircuitOpenException(SecondsUntil(_openUntil!.Value, now));
                case AiCircuitState.HalfOpen when _probeInFlight:
                    throw new AiCircuitOpenException(1);
                case AiCircuitState.HalfOpen:
                    _probeInFlight = true;
                    break;
            }
        }
    }

    /// <summary>The provider answered.</summary>
    public virtual void RecordSuccess()
    {
        lock (_sync)
        {
            if (CurrentState(_time.GetUtcNow()) == AiCircuitState.Open)
            {
                // A call that started before the breaker opened does not end the pause.
                return;
            }

            Close();
        }
    }

    /// <summary>The provider call failed (after the provider SDK's own retries).</summary>
    public virtual void RecordFailure(AiProviderErrorKind kind)
    {
        lock (_sync)
        {
            var now = _time.GetUtcNow();
            if (kind == AiProviderErrorKind.Auth)
            {
                var wasPausedForAuth = _openUntil.HasValue && _openUntil.Value - now > OpenDuration;
                Open(now, AuthOpenDuration);
                if (!wasPausedForAuth)
                {
                    _logger.LogCritical(
                        "AI provider rejected credentials; AI requests are paused for {Minutes} minutes.",
                        (int)AuthOpenDuration.TotalMinutes);
                }

                return;
            }

            if (!AiProviderErrorMapper.IsTransient(kind))
            {
                // The provider answered (it refused this request): it is reachable.
                if (CurrentState(now) != AiCircuitState.Open)
                {
                    Close();
                }

                return;
            }

            if (_probeInFlight)
            {
                Open(now, OpenDuration);
                return;
            }

            if (_failures == 0 || now - _firstFailureAt > FailureWindow)
            {
                _failures = 1;
                _firstFailureAt = now;
            }
            else
            {
                _failures++;
            }

            if (_failures >= FailureThreshold && CurrentState(now) == AiCircuitState.Closed)
            {
                Open(now, OpenDuration);
                _logger.LogWarning(
                    "AI provider failed {Failures} times in a row; AI requests are paused for {Seconds} seconds.",
                    _failures, (int)OpenDuration.TotalSeconds);
            }
        }
    }

    /// <summary>A call let through ended without telling anything about the provider (the caller went away).</summary>
    public virtual void RecordAbandoned()
    {
        lock (_sync)
        {
            _probeInFlight = false;
        }
    }

    private AiCircuitState CurrentState(DateTimeOffset now)
    {
        if (!_openUntil.HasValue)
        {
            return AiCircuitState.Closed;
        }

        return now < _openUntil.Value ? AiCircuitState.Open : AiCircuitState.HalfOpen;
    }

    private void Open(DateTimeOffset now, TimeSpan duration)
    {
        var until = now + duration;
        _openUntil = _openUntil.HasValue && _openUntil.Value > until ? _openUntil : until;
        _probeInFlight = false;
        _failures = 0;
    }

    private void Close()
    {
        _openUntil = null;
        _probeInFlight = false;
        _failures = 0;
    }

    private static int SecondsUntil(DateTimeOffset until, DateTimeOffset now) =>
        Math.Max(1, (int)Math.Ceiling((until - now).TotalSeconds));
}
