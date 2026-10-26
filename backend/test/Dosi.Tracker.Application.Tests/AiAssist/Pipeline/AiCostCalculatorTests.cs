using System.Collections.Generic;
using Microsoft.Extensions.Options;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

public class AiCostCalculatorTests
{
    private static AiCostCalculator Calculator(params AiModelPricing[] prices) =>
        new(Options.Create(new AiOptions { Pricing = new List<AiModelPricing>(prices) }));

    private static readonly AiModelPricing Mini = new()
    {
        Model = "gpt-5.4-mini", InputPer1M = 0.75m, CachedInputPer1M = 0.075m, OutputPer1M = 4.5m
    };

    [Fact]
    public void Cached_Input_Should_Be_Priced_At_Its_Own_Rate_And_Reasoning_Not_Added_Again()
    {
        // (2000 - 1000) × 0.75 + 1000 × 0.075 + 500 × 4.5 = 750 + 75 + 2250 per million.
        Calculator(Mini).Estimate("gpt-5.4-mini", 2000, 1000, 500).ShouldBe(0.003075m);
    }

    [Fact]
    public void Cached_Input_Without_A_Cached_Price_Should_Be_Priced_As_Input()
    {
        var price = new AiModelPricing { Model = "m", InputPer1M = 1m, OutputPer1M = 2m };

        AiCostCalculator.Calculate(price, 1000, 400, 1000).ShouldBe(0.003m);
    }

    [Fact]
    public void A_Model_Without_A_Price_Should_Have_No_Estimate()
    {
        Calculator(Mini).Estimate("gpt-4.1-mini", 1000, 0, 100).ShouldBeNull();
        Calculator().Estimate("gpt-5.4-mini", 1000, 0, 100).ShouldBeNull();
        Calculator(Mini).Estimate(null, 1000, 0, 100).ShouldBeNull();
        Calculator(Mini).Estimate("  ", 1000, 0, 100).ShouldBeNull();
    }

    [Fact]
    public void A_Dated_Snapshot_Should_Take_The_Price_Of_The_Model_It_Extends()
    {
        var calculator = Calculator(
            new AiModelPricing { Model = "gpt-5.4", InputPer1M = 10m, OutputPer1M = 10m },
            Mini);

        calculator.FindPricing("gpt-5.4-mini-2026-03-17").ShouldBeSameAs(Mini);
        calculator.FindPricing("GPT-5.4-MINI").ShouldBeSameAs(Mini);
        calculator.FindPricing("gpt-5.4-2026-01-01")!.InputPer1M.ShouldBe(10m);
        calculator.FindPricing("gpt-5.4o").ShouldBeNull();
    }

    [Fact]
    public void Unknown_Token_Counts_Should_Have_No_Estimate_And_Odd_Counts_Should_Be_Bounded()
    {
        AiCostCalculator.Calculate(Mini, null, null, null).ShouldBeNull();
        AiCostCalculator.Calculate(Mini, 1000, null, null).ShouldBe(0.00075m);
        AiCostCalculator.Calculate(Mini, null, null, 1000).ShouldBe(0.0045m);
        // More cached input than input is read as all input cached; negative counts as none.
        AiCostCalculator.Calculate(Mini, 1000, 5000, -3).ShouldBe(0.000075m);
    }

    [Fact]
    public void The_Estimate_Should_Be_Rounded_To_The_Ledgers_Precision()
    {
        var price = new AiModelPricing { Model = "m", InputPer1M = 0.1m, OutputPer1M = 0m };

        AiCostCalculator.Calculate(price, 7, 0, 0).ShouldBe(0.000001m);
        AiCostCalculator.Calculate(price, 4, 0, 0).ShouldBe(0m);
    }
}
