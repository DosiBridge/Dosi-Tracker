using System;
using System.Collections.Generic;
using Microsoft.Extensions.Options;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

public class AiQuotaPolicyTests
{
    private static AiQuotaPolicy Policy(AiQuotaOptions? quotas = null) =>
        new(Options.Create(new AiOptions { Quotas = quotas ?? new AiQuotaOptions() }));

    [Theory]
    [InlineData("Free", 50, 200_000L, 10, 3)]
    [InlineData("starter", 500, 2_000_000L, 30, 5)]
    [InlineData("BUSINESS", 3000, 10_000_000L, 60, 6)]
    [InlineData(" Business ", 3000, 10_000_000L, 60, 6)]
    public void A_Known_Plan_Should_Get_Its_Allowance_Whatever_The_Case(string plan, int requests, long tokens, int daily, int perMinute)
    {
        var limits = Policy().For(plan);

        limits.MonthlyRequests.ShouldBe(requests);
        limits.MonthlyTokens.ShouldBe(tokens);
        limits.UserDailyRequests.ShouldBe(daily);
        limits.UserPerMinute.ShouldBe(perMinute);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("Default")]
    [InlineData("Enterprise")]
    public void Anything_Else_Should_Get_The_Default(string? plan)
    {
        var limits = Policy().For(plan);

        limits.MonthlyRequests.ShouldBe(200);
        limits.MonthlyTokens.ShouldBe(800_000);
        limits.UserDailyRequests.ShouldBe(20);
        limits.UserPerMinute.ShouldBe(4);
    }

    [Fact]
    public void A_Plan_Dictionary_Replaced_In_Code_Should_Still_Ignore_Case()
    {
        var quotas = new AiQuotaOptions
        {
            Plans = new Dictionary<string, AiQuotaLimits>(StringComparer.Ordinal) { ["Team"] = new() { MonthlyRequests = 7 } }
        };

        Policy(quotas).For("team").MonthlyRequests.ShouldBe(7);
    }

    [Fact]
    public void Null_Should_Mean_Unlimited_And_A_Negative_Limit_Nothing()
    {
        var quotas = new AiQuotaOptions
        {
            Default = new AiQuotaLimits { MonthlyRequests = null, MonthlyTokens = -1, UserDailyRequests = -5, UserPerMinute = null }
        };

        var limits = Policy(quotas).For(null);

        limits.MonthlyRequests.ShouldBeNull();
        limits.MonthlyTokens.ShouldBe(0);
        limits.UserDailyRequests.ShouldBe(0);
        limits.UserPerMinute.ShouldBeNull();
    }

    [Fact]
    public void The_Result_Should_Be_A_Copy()
    {
        var policy = Policy();
        policy.For("Free").MonthlyRequests = 1;

        policy.For("Free").MonthlyRequests.ShouldBe(50);
    }
}
