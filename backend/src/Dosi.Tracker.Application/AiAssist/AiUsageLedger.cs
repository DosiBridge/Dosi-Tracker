using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;

namespace Dosi.Tracker.AiAssist;

/// <summary>Requests and tokens summed over the usage ledger.</summary>
public readonly record struct AiUsageTotals(int Requests, long Tokens);

/// <summary>One workspace's month on the platform overview.</summary>
public sealed record AiWorkspaceMonthUsage(
    Guid WorkspaceId,
    long Requests,
    long InputTokens,
    long OutputTokens,
    long Fallbacks,
    long Errors,
    decimal? EstimatedCostUsd);

/// <summary>
/// Sums over the usage ledger (<see cref="AiUsageRecord"/>) for the allowances, the usage views and the platform
/// overview. Only rows that reached the provider count (<see cref="AiUsageRecordQueries.WhereCalledProvider"/>); tokens
/// are input plus output. Costs are summed in memory over the priced rows only: SQLite, which the tests run on, cannot
/// sum decimals.
/// </summary>
public class AiUsageLedger : ITransientDependency
{
    private readonly IRepository<AiUsageRecord, Guid> _repository;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly IDataFilter _dataFilter;

    public AiUsageLedger(IRepository<AiUsageRecord, Guid> repository, IAsyncQueryableExecuter asyncExecuter, IDataFilter dataFilter)
    {
        _repository = repository;
        _asyncExecuter = asyncExecuter;
        _dataFilter = dataFilter;
    }

    /// <summary>The workspace's provider requests since <paramref name="since"/>, optionally only one member's.</summary>
    public virtual async Task<AiUsageTotals> GetTotalsAsync(Guid workspaceId, DateTime since, Guid? userId = null)
    {
        var query = (await CalledSinceAsync(since)).Where(r => r.WorkspaceId == workspaceId);
        if (userId.HasValue)
        {
            var id = userId.Value;
            query = query.Where(r => r.UserId == id);
        }

        var count = await _asyncExecuter.CountAsync(query);
        var tokens = count == 0 ? 0L : await _asyncExecuter.SumAsync(query.Select(r => (long)(r.InputTokens ?? 0) + (r.OutputTokens ?? 0)));
        return new AiUsageTotals(count, tokens);
    }

    /// <summary>The workspace's provider requests since <paramref name="since"/> per feature, ordered by feature id.</summary>
    public virtual async Task<List<(string FeatureId, AiUsageTotals Totals)>> GetByFeatureAsync(Guid workspaceId, DateTime since)
    {
        var rows = await _asyncExecuter.ToListAsync(
            (await CalledSinceAsync(since))
                .Where(r => r.WorkspaceId == workspaceId)
                .GroupBy(r => r.FeatureId)
                .Select(g => new
                {
                    FeatureId = g.Key,
                    Requests = g.Count(),
                    Tokens = g.Sum(r => (long)(r.InputTokens ?? 0) + (r.OutputTokens ?? 0))
                }));

        return rows
            .OrderBy(r => r.FeatureId, StringComparer.Ordinal)
            .Select(r => (r.FeatureId, new AiUsageTotals(r.Requests, r.Tokens)))
            .ToList();
    }

    /// <summary>The workspace's estimated cost since <paramref name="since"/>; null when no request had a price.</summary>
    public virtual async Task<decimal?> GetCostAsync(Guid workspaceId, DateTime since)
    {
        var costs = await _asyncExecuter.ToListAsync(
            (await _repository.GetQueryableAsync())
                .Where(r => r.WorkspaceId == workspaceId && r.CreationTime >= since && r.EstimatedCostUsd != null)
                .Select(r => r.EstimatedCostUsd!.Value));
        return costs.Count == 0 ? null : costs.Sum();
    }

    /// <summary>Every workspace's provider requests since <paramref name="since"/> (the workspace filter is off).</summary>
    public virtual async Task<List<AiWorkspaceMonthUsage>> GetPlatformAsync(DateTime since)
    {
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            var rows = await _asyncExecuter.ToListAsync(
                (await CalledSinceAsync(since))
                    .GroupBy(r => r.WorkspaceId)
                    .Select(g => new
                    {
                        WorkspaceId = g.Key,
                        Requests = g.LongCount(),
                        InputTokens = g.Sum(r => (long)(r.InputTokens ?? 0)),
                        OutputTokens = g.Sum(r => (long)(r.OutputTokens ?? 0)),
                        Fallbacks = g.LongCount(r => r.Outcome == AiUsageOutcomes.Fallback),
                        Errors = g.LongCount(r => r.Outcome == AiUsageOutcomes.ProviderError || r.Outcome == AiUsageOutcomes.Timeout)
                    }));

            var costs = (await _asyncExecuter.ToListAsync(
                    (await _repository.GetQueryableAsync())
                        .Where(r => r.CreationTime >= since && r.EstimatedCostUsd != null)
                        .Select(r => new { r.WorkspaceId, Cost = r.EstimatedCostUsd!.Value })))
                .GroupBy(r => r.WorkspaceId)
                .ToDictionary(g => g.Key, g => g.Sum(r => r.Cost));

            return rows
                .Select(r => new AiWorkspaceMonthUsage(
                    r.WorkspaceId, r.Requests, r.InputTokens, r.OutputTokens, r.Fallbacks, r.Errors,
                    costs.TryGetValue(r.WorkspaceId, out var cost) ? cost : null))
                .ToList();
        }
    }

    private async Task<IQueryable<AiUsageRecord>> CalledSinceAsync(DateTime since)
    {
        return (await _repository.GetQueryableAsync())
            .Where(r => r.CreationTime >= since)
            .WhereCalledProvider();
    }
}
