using System;
using System.Linq;
using Microsoft.Extensions.Options;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// The AI allowance of a workspace: <c>AI:Quotas:Plans:{PlanName}</c> for its plan (the name compared ignoring case),
/// else <c>AI:Quotas:Default</c>. Null means unlimited; a negative value (a configuration mistake) allows nothing,
/// so a typo never lifts a limit.
/// </summary>
public class AiQuotaPolicy : ITransientDependency
{
    private readonly IOptions<AiOptions> _options;

    public AiQuotaPolicy(IOptions<AiOptions> options)
    {
        _options = options;
    }

    public virtual AiQuotaLimits For(string? planName)
    {
        var quotas = _options.Value.Quotas;
        var limits = Find(quotas, planName) ?? quotas.Default ?? new AiQuotaLimits();

        return new AiQuotaLimits
        {
            MonthlyRequests = NotNegative(limits.MonthlyRequests),
            MonthlyTokens = NotNegative(limits.MonthlyTokens),
            UserDailyRequests = NotNegative(limits.UserDailyRequests),
            UserPerMinute = NotNegative(limits.UserPerMinute)
        };
    }

    private static AiQuotaLimits? Find(AiQuotaOptions quotas, string? planName)
    {
        var name = planName?.Trim();
        if (string.IsNullOrEmpty(name) || quotas.Plans == null)
        {
            return null;
        }

        // The dictionary ignores case as bound; the scan keeps that true for a dictionary replaced in code.
        if (quotas.Plans.TryGetValue(name, out var limits) && limits != null)
        {
            return limits;
        }

        return quotas.Plans.FirstOrDefault(p => string.Equals(p.Key, name, StringComparison.OrdinalIgnoreCase)).Value;
    }

    private static int? NotNegative(int? value) => value is < 0 ? 0 : value;

    private static long? NotNegative(long? value) => value is < 0 ? 0 : value;
}
