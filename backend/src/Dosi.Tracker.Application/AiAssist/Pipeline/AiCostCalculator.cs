using System;
using System.Linq;
using Microsoft.Extensions.Options;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// The estimated USD cost of one request from <c>AI:Pricing</c>:
/// <c>((input − cachedInput) × InputPer1M + cachedInput × CachedInputPer1M + output × OutputPer1M) / 1e6</c>, rounded to
/// six decimals (the ledger's precision). Reasoning tokens are already part of the output for OpenAI, so they are
/// not added again. A model without a price has no estimate (null); its tokens are still recorded.
/// </summary>
public class AiCostCalculator : ITransientDependency
{
    private const decimal TokensPerUnit = 1_000_000m;

    private readonly IOptions<AiOptions> _options;

    public AiCostCalculator(IOptions<AiOptions> options)
    {
        _options = options;
    }

    /// <param name="model">The model that answered: its configured id ("gpt-5.4-mini") or the dated snapshot the
    /// provider reports ("gpt-5.4-mini-2026-03-17"), which takes the price of the longest configured id it extends.</param>
    public virtual decimal? Estimate(string? model, int? inputTokens, int? cachedInputTokens, int? outputTokens)
    {
        var pricing = FindPricing(model);
        return pricing == null ? null : Calculate(pricing, inputTokens, cachedInputTokens, outputTokens);
    }

    /// <summary>The configured price of <paramref name="model"/>, or null.</summary>
    public virtual AiModelPricing? FindPricing(string? model)
    {
        var id = model?.Trim();
        if (string.IsNullOrEmpty(id))
        {
            return null;
        }

        var prices = (_options.Value.Pricing ?? new()).Where(p => p != null && !string.IsNullOrWhiteSpace(p.Model)).ToList();
        return prices.FirstOrDefault(p => string.Equals(p.Model.Trim(), id, StringComparison.OrdinalIgnoreCase))
               ?? prices
                   .Where(p => id.StartsWith(p.Model.Trim() + "-", StringComparison.OrdinalIgnoreCase))
                   .OrderByDescending(p => p.Model.Trim().Length)
                   .FirstOrDefault();
    }

    /// <summary>The cost of one request at <paramref name="pricing"/>; null when no token count is known. Cached input
    /// without a cached price is priced as input.</summary>
    public static decimal? Calculate(AiModelPricing pricing, int? inputTokens, int? cachedInputTokens, int? outputTokens)
    {
        if (inputTokens == null && outputTokens == null)
        {
            return null;
        }

        var input = Math.Max(0, inputTokens ?? 0);
        var cached = Math.Min(Math.Max(0, cachedInputTokens ?? 0), input);
        var output = Math.Max(0, outputTokens ?? 0);

        var cost = ((input - cached) * pricing.InputPer1M
                    + cached * (pricing.CachedInputPer1M ?? pricing.InputPer1M)
                    + output * pricing.OutputPer1M) / TokensPerUnit;

        return Math.Round(Math.Max(0m, cost), 6, MidpointRounding.AwayFromZero);
    }
}
