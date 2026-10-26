using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Microsoft.Extensions.Caching.Memory;
using Microsoft.Extensions.Options;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Timing;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>The platform's usage this month, every workspace together.</summary>
public readonly record struct AiPlatformMonthUsage(long Tokens, decimal? EstimatedCostUsd);

/// <summary>
/// The AI allowances (spec 14), all counted from the usage ledger, so refusals (which write no row) never count:
/// <list type="bullet">
/// <item>per minute, every row of the member counts, cache hits included (<see cref="EnsureBurstAsync"/>);</item>
/// <item>per member and UTC day, per workspace and UTC month, and for the whole platform per month, only rows that
/// reached the provider count, and tokens are input plus output (<see cref="EnsureQuotaAsync"/>);</item>
/// <item>a member may ask for a fresh answer instead of the cached one <see cref="MaxRegeneratesPerDay"/> times a day
/// per feature and subject (<see cref="RegenerateAllowedAsync"/>).</item>
/// </list>
/// Limits come from <see cref="AiQuotaPolicy"/> (by plan) and <c>AI:Budget</c> (the platform).
/// </summary>
public class AiUsageGate : ITransientDependency
{
    public const int MaxRegeneratesPerDay = 3;

    /// <summary>How long the platform's month total is reused before it is summed again.</summary>
    public static readonly TimeSpan PlatformTotalsLifetime = TimeSpan.FromSeconds(60);

    private static readonly TimeSpan Minute = TimeSpan.FromMinutes(1);

    private readonly IRepository<AiUsageRecord, Guid> _repository;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly AiUsageLedger _ledger;
    private readonly AiQuotaPolicy _quotaPolicy;
    private readonly IOptions<AiOptions> _options;
    private readonly IMemoryCache _memoryCache;
    private readonly IClock _clock;

    public AiUsageGate(
        IRepository<AiUsageRecord, Guid> repository,
        IAsyncQueryableExecuter asyncExecuter,
        AiUsageLedger ledger,
        AiQuotaPolicy quotaPolicy,
        IOptions<AiOptions> options,
        IMemoryCache memoryCache,
        IClock clock)
    {
        _repository = repository;
        _asyncExecuter = asyncExecuter;
        _ledger = ledger;
        _quotaPolicy = quotaPolicy;
        _options = options;
        _memoryCache = memoryCache;
        _clock = clock;
    }

    /// <summary>Refuses (<c>Tracker:AiRateLimited</c> minute) a member who made the plan's per-minute number of
    /// requests in the last minute, with the seconds until the oldest of them leaves the window.</summary>
    public virtual async Task EnsureBurstAsync(AiCallerContext caller, string featureId)
    {
        var limit = _quotaPolicy.For(caller.PlanName).UserPerMinute;
        if (limit == null)
        {
            return;
        }

        var now = _clock.Now;
        var since = now - Minute;
        var workspaceId = caller.WorkspaceId;
        var userId = caller.UserId;
        var times = await _asyncExecuter.ToListAsync(
            (await _repository.GetQueryableAsync())
                .Where(r => r.WorkspaceId == workspaceId && r.UserId == userId && r.CreationTime > since)
                .OrderByDescending(r => r.CreationTime)
                .Select(r => r.CreationTime)
                .Take(Math.Max(1, limit.Value)));

        if (times.Count < limit.Value)
        {
            return;
        }

        // The window frees up when the oldest of the last `limit` requests is a minute old.
        var frees = times.Count == 0 ? now : times.Min() + Minute;
        throw AiErrors.RateLimited(AiErrors.MinuteReason, (int)Math.Ceiling((frees - now).TotalSeconds), featureId);
    }

    /// <summary>
    /// Refuses (<c>Tracker:AiQuotaExceeded</c>) a request the allowances cannot take: the member's daily requests, the
    /// workspace's monthly requests, or monthly tokens (counting <paramref name="estimatedTokens"/> for this request),
    /// then the platform's monthly token and USD budget.
    /// </summary>
    public virtual async Task EnsureQuotaAsync(AiCallerContext caller, string featureId, long estimatedTokens)
    {
        var limits = _quotaPolicy.For(caller.PlanName);
        var now = _clock.Now;
        var monthStart = AiUsageRecordQueries.MonthStart(now);
        var nextMonth = monthStart.AddMonths(1);
        var estimate = Math.Max(0L, estimatedTokens);

        if (limits.UserDailyRequests.HasValue)
        {
            var dayStart = AiUsageRecordQueries.DayStart(now);
            var today = await _ledger.GetTotalsAsync(caller.WorkspaceId, dayStart, caller.UserId);
            if (today.Requests >= limits.UserDailyRequests.Value)
            {
                throw AiErrors.QuotaExceeded(AiErrors.UserDayScope, dayStart.AddDays(1), featureId);
            }
        }

        if (limits.MonthlyRequests.HasValue || limits.MonthlyTokens.HasValue)
        {
            var month = await _ledger.GetTotalsAsync(caller.WorkspaceId, monthStart);
            if (month.Requests >= limits.MonthlyRequests.GetValueOrDefault(int.MaxValue) ||
                month.Tokens + estimate > limits.MonthlyTokens.GetValueOrDefault(long.MaxValue))
            {
                throw AiErrors.QuotaExceeded(AiErrors.WorkspaceMonthScope, nextMonth, featureId);
            }
        }

        var budget = _options.Value.Budget ?? new AiBudgetOptions();
        if (budget.MonthlyTokenLimit.HasValue || budget.MonthlyUsdLimit.HasValue)
        {
            var platform = await GetPlatformMonthAsync(monthStart);
            if (platform.Tokens + estimate > budget.MonthlyTokenLimit.GetValueOrDefault(long.MaxValue) ||
                (budget.MonthlyUsdLimit.HasValue && platform.EstimatedCostUsd >= budget.MonthlyUsdLimit.Value))
            {
                throw AiErrors.QuotaExceeded(AiErrors.PlatformMonthScope, nextMonth, featureId);
            }
        }
    }

    /// <summary>Whether the member may still skip the cache today for this feature and subject (null: no single
    /// member, such as a team summary).</summary>
    public virtual async Task<bool> RegenerateAllowedAsync(AiCallerContext caller, string featureId, Guid? subjectUserId)
    {
        var dayStart = AiUsageRecordQueries.DayStart(_clock.Now);
        var workspaceId = caller.WorkspaceId;
        var userId = caller.UserId;
        var query = (await _repository.GetQueryableAsync())
            .Where(r => r.WorkspaceId == workspaceId && r.UserId == userId && r.FeatureId == featureId &&
                        r.IsRegenerate && r.CreationTime >= dayStart);
        query = subjectUserId.HasValue
            ? query.Where(r => r.SubjectUserId == subjectUserId.Value)
            : query.Where(r => r.SubjectUserId == null);

        return await _asyncExecuter.CountAsync(query) < MaxRegeneratesPerDay;
    }

    /// <summary>The platform's provider tokens and priced cost this month, summed at most every
    /// <see cref="PlatformTotalsLifetime"/> per instance.</summary>
    public virtual async Task<AiPlatformMonthUsage> GetPlatformMonthAsync(DateTime monthStart)
    {
        var key = "Tracker:Ai:PlatformMonth:" + monthStart.Ticks;
        if (_memoryCache.TryGetValue(key, out AiPlatformMonthUsage cached))
        {
            return cached;
        }

        var usage = await _ledger.GetPlatformAsync(monthStart);
        var costs = usage.Where(u => u.EstimatedCostUsd.HasValue).Select(u => u.EstimatedCostUsd!.Value).ToList();
        var totals = new AiPlatformMonthUsage(
            usage.Sum(u => u.InputTokens + u.OutputTokens),
            costs.Count == 0 ? null : costs.Sum());

        _memoryCache.Set(key, totals, PlatformTotalsLifetime);
        return totals;
    }
}
