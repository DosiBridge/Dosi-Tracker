using System;
using Dosi.Tracker.AiAssist.Fakes;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.Logging;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>A clock tests move by hand.</summary>
public sealed class ManualTimeProvider : TimeProvider
{
    private DateTimeOffset _now = new(2026, 10, 26, 9, 0, 0, TimeSpan.Zero);

    public override DateTimeOffset GetUtcNow() => _now;

    public void Advance(TimeSpan by) => _now += by;
}

public class AiCircuitBreakerTests
{
    private readonly ManualTimeProvider _time = new();
    private readonly CapturingLoggerProvider _logs = new();
    private readonly AiCircuitBreaker _breaker;

    public AiCircuitBreakerTests()
    {
        _breaker = new AiCircuitBreaker(new LoggerFactory(new[] { _logs }).CreateLogger<AiCircuitBreaker>(), _time);
    }

    private void Fail(int times, AiProviderErrorKind kind = AiProviderErrorKind.ServerError)
    {
        for (var i = 0; i < times; i++)
        {
            _breaker.ThrowIfOpen();
            _breaker.RecordFailure(kind);
        }
    }

    [Fact]
    public void A_New_Breaker_Should_Be_Closed_And_Let_Calls_Through()
    {
        _breaker.State.ShouldBe(AiCircuitState.Closed);
        _breaker.StateName.ShouldBe("closed");
        Should.NotThrow(() => _breaker.ThrowIfOpen());
    }

    [Fact]
    public void Five_Transient_Failures_In_A_Row_Should_Open_It_For_A_Minute()
    {
        Fail(AiCircuitBreaker.FailureThreshold - 1);
        _breaker.State.ShouldBe(AiCircuitState.Closed);

        Fail(1, AiProviderErrorKind.Timeout);

        _breaker.State.ShouldBe(AiCircuitState.Open);
        _breaker.StateName.ShouldBe("open");
        var open = Should.Throw<AiCircuitOpenException>(() => _breaker.ThrowIfOpen());
        open.RetryAfterSeconds.ShouldBe(60);

        _time.Advance(TimeSpan.FromSeconds(45));
        Should.Throw<AiCircuitOpenException>(() => _breaker.ThrowIfOpen()).RetryAfterSeconds.ShouldBe(15);
        _logs.Entries.ShouldContain(e => e.Level == LogLevel.Warning);
    }

    [Fact]
    public void A_Success_Should_Reset_The_Count()
    {
        Fail(AiCircuitBreaker.FailureThreshold - 1);
        _breaker.RecordSuccess();
        Fail(AiCircuitBreaker.FailureThreshold - 1);

        _breaker.State.ShouldBe(AiCircuitState.Closed);
    }

    [Fact]
    public void Failures_Spread_Over_More_Than_The_Window_Should_Not_Open_It()
    {
        Fail(AiCircuitBreaker.FailureThreshold - 1);
        _time.Advance(AiCircuitBreaker.FailureWindow + TimeSpan.FromSeconds(1));
        Fail(1);

        _breaker.State.ShouldBe(AiCircuitState.Closed);
    }

    [Theory]
    [InlineData(AiProviderErrorKind.BadRequest)]
    [InlineData(AiProviderErrorKind.ContentFilter)]
    public void A_Refused_Request_Should_Count_As_A_Reachable_Provider(AiProviderErrorKind kind)
    {
        Fail(AiCircuitBreaker.FailureThreshold - 1);
        Fail(1, kind);
        Fail(AiCircuitBreaker.FailureThreshold - 1);

        _breaker.State.ShouldBe(AiCircuitState.Closed);
    }

    [Fact]
    public void After_The_Pause_One_Probe_Should_Go_Through_And_Close_It_On_Success()
    {
        Fail(AiCircuitBreaker.FailureThreshold);
        _time.Advance(AiCircuitBreaker.OpenDuration);

        _breaker.State.ShouldBe(AiCircuitState.HalfOpen);
        _breaker.StateName.ShouldBe("half-open");
        Should.NotThrow(() => _breaker.ThrowIfOpen());
        Should.Throw<AiCircuitOpenException>(() => _breaker.ThrowIfOpen()).RetryAfterSeconds.ShouldBe(1);

        _breaker.RecordSuccess();

        _breaker.State.ShouldBe(AiCircuitState.Closed);
        Should.NotThrow(() => _breaker.ThrowIfOpen());
    }

    [Fact]
    public void A_Failed_Probe_Should_Open_It_Again()
    {
        Fail(AiCircuitBreaker.FailureThreshold);
        _time.Advance(AiCircuitBreaker.OpenDuration);

        Fail(1);

        _breaker.State.ShouldBe(AiCircuitState.Open);
        Should.Throw<AiCircuitOpenException>(() => _breaker.ThrowIfOpen()).RetryAfterSeconds.ShouldBe(60);
    }

    [Fact]
    public void An_Abandoned_Probe_Should_Let_The_Next_Request_Probe()
    {
        Fail(AiCircuitBreaker.FailureThreshold);
        _time.Advance(AiCircuitBreaker.OpenDuration);
        _breaker.ThrowIfOpen();

        _breaker.RecordAbandoned();

        Should.NotThrow(() => _breaker.ThrowIfOpen());
    }

    [Fact]
    public void A_Late_Success_Should_Not_End_The_Pause()
    {
        Fail(AiCircuitBreaker.FailureThreshold);

        _breaker.RecordSuccess();

        _breaker.State.ShouldBe(AiCircuitState.Open);
    }

    [Fact]
    public void A_Rejected_Key_Should_Open_It_For_Ten_Minutes_And_Log_Once_Without_Detail()
    {
        Fail(1, AiProviderErrorKind.Auth);

        _breaker.State.ShouldBe(AiCircuitState.Open);
        Should.Throw<AiCircuitOpenException>(() => _breaker.ThrowIfOpen()).RetryAfterSeconds.ShouldBe(600);

        _breaker.RecordFailure(AiProviderErrorKind.Auth);
        var critical = _logs.Entries.ShouldHaveSingleItem();
        critical.Level.ShouldBe(LogLevel.Critical);
        critical.Message.ShouldBe("AI provider rejected credentials; AI requests are paused for 10 minutes.");

        _time.Advance(TimeSpan.FromMinutes(9));
        _breaker.State.ShouldBe(AiCircuitState.Open);
        _time.Advance(TimeSpan.FromMinutes(1));
        _breaker.State.ShouldBe(AiCircuitState.HalfOpen);
    }

    [Fact]
    public void Transient_Failures_During_A_Key_Pause_Should_Not_Shorten_It()
    {
        Fail(1, AiProviderErrorKind.Auth);
        _breaker.RecordFailure(AiProviderErrorKind.ServerError);

        _time.Advance(TimeSpan.FromMinutes(5));

        _breaker.State.ShouldBe(AiCircuitState.Open);
    }
}
