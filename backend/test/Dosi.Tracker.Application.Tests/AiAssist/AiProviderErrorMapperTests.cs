using System;
using Dosi.Tracker.AiAssist.Provider;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>AiProviderErrorMapper: provider statuses to error kinds, what is retried, and Retry-After.</summary>
public class AiProviderErrorMapperTests
{
    [Theory]
    [InlineData(401, AiProviderErrorKind.Auth)]
    [InlineData(403, AiProviderErrorKind.Auth)]
    [InlineData(408, AiProviderErrorKind.Timeout)]
    [InlineData(429, AiProviderErrorKind.RateLimited)]
    [InlineData(400, AiProviderErrorKind.BadRequest)]
    [InlineData(404, AiProviderErrorKind.BadRequest)]
    [InlineData(422, AiProviderErrorKind.BadRequest)]
    [InlineData(500, AiProviderErrorKind.ServerError)]
    [InlineData(503, AiProviderErrorKind.ServerError)]
    [InlineData(520, AiProviderErrorKind.ServerError)]
    [InlineData(302, AiProviderErrorKind.ServerError)]
    [InlineData(0, AiProviderErrorKind.Network)]
    [InlineData(null, AiProviderErrorKind.Network)]
    public void Statuses_Map_To_Kinds(int? status, AiProviderErrorKind kind)
    {
        AiProviderErrorMapper.Map(status).ShouldBe(kind);
    }

    [Theory]
    [InlineData(408, true)]
    [InlineData(429, true)]
    [InlineData(500, true)]
    [InlineData(520, true)]
    [InlineData(529, true)]
    [InlineData(599, true)]
    [InlineData(400, false)]
    [InlineData(401, false)]
    [InlineData(404, false)]
    [InlineData(200, false)]
    public void Timeouts_Rate_Limits_And_Every_5xx_Are_Retried(int status, bool retriable)
    {
        AiProviderErrorMapper.IsRetriableStatus(status).ShouldBe(retriable);
    }

    [Theory]
    [InlineData(AiProviderErrorKind.Timeout, true, "timeout")]
    [InlineData(AiProviderErrorKind.RateLimited, true, "rate-limited")]
    [InlineData(AiProviderErrorKind.ServerError, true, "error")]
    [InlineData(AiProviderErrorKind.Network, true, "error")]
    [InlineData(AiProviderErrorKind.Auth, false, "error")]
    [InlineData(AiProviderErrorKind.BadRequest, false, "error")]
    [InlineData(AiProviderErrorKind.ContentFilter, false, "error")]
    public void Kinds_Are_Transient_Or_Not_And_Have_A_Wire_Kind(AiProviderErrorKind kind, bool transient, string wireKind)
    {
        AiProviderErrorMapper.IsTransient(kind).ShouldBe(transient);
        new AiProviderException(kind).IsTransient.ShouldBe(transient);
        AiProviderErrorMapper.ToUnavailableKind(kind).ShouldBe(wireKind);
    }

    [Theory]
    [InlineData("12", 12)]
    [InlineData(" 1.2 ", 2)]
    [InlineData("0", 0)]
    [InlineData("999999", AiProviderErrorMapper.MaxRetryAfterSeconds)]
    [InlineData("-5", null)]
    [InlineData("soon", null)]
    [InlineData("", null)]
    [InlineData(null, null)]
    public void Retry_After_Seconds_Are_Read(string? value, int? seconds)
    {
        AiProviderErrorMapper.ParseRetryAfterSeconds(value, DateTimeOffset.UtcNow).ShouldBe(seconds);
    }

    [Fact]
    public void Retry_After_Dates_Are_Read_Relative_To_Now()
    {
        var now = new DateTimeOffset(2026, 10, 26, 12, 0, 0, TimeSpan.Zero);

        AiProviderErrorMapper.ParseRetryAfterSeconds("Mon, 26 Oct 2026 12:00:30 GMT", now).ShouldBe(30);
        AiProviderErrorMapper.ParseRetryAfterSeconds("Mon, 26 Oct 2026 11:59:00 GMT", now).ShouldBe(0);
        AiProviderErrorMapper.ParseRetryAfterSeconds("Tue, 27 Oct 2026 12:00:00 GMT", now)
            .ShouldBe(AiProviderErrorMapper.MaxRetryAfterSeconds);
    }

    [Fact]
    public void The_Exception_Carries_No_Provider_Text()
    {
        var exception = new AiProviderException(AiProviderErrorKind.RateLimited, 429, 20);

        exception.Kind.ShouldBe(AiProviderErrorKind.RateLimited);
        exception.Status.ShouldBe(429);
        exception.RetryAfterSeconds.ShouldBe(20);
        exception.InnerException.ShouldBeNull();
        exception.Message.ShouldBe("The AI provider call failed: RateLimited (HTTP 429).");
        new AiProviderException(AiProviderErrorKind.Network).Message.ShouldBe("The AI provider call failed: Network.");
    }
}
