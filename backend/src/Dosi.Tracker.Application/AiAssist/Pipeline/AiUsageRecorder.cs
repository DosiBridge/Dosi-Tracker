using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Uow;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>Tokens a request used, summed over its provider calls; null counts are unknown.</summary>
public readonly record struct AiTokenUsage(int? InputTokens, int? OutputTokens, int? CachedInputTokens, int? ReasoningTokens)
{
    public static readonly AiTokenUsage None = new(null, null, null, null);

    public bool IsEmpty => InputTokens == null && OutputTokens == null;

    /// <summary>This usage plus what one provider answer reported.</summary>
    public AiTokenUsage Add(UsageDetails? usage)
    {
        if (usage == null)
        {
            return this;
        }

        return new AiTokenUsage(
            Sum(InputTokens, usage.InputTokenCount),
            Sum(OutputTokens, usage.OutputTokenCount),
            Sum(CachedInputTokens, usage.CachedInputTokenCount),
            Sum(ReasoningTokens, usage.ReasoningTokenCount));
    }

    private static int? Sum(int? total, long? more)
    {
        if (more == null || more < 0)
        {
            return total;
        }

        var sum = (total ?? 0L) + more.Value;
        return sum > int.MaxValue ? int.MaxValue : (int)sum;
    }
}

/// <summary>One ledger row to write (<see cref="AiUsageRecorder.RecordAsync"/>).</summary>
public sealed record AiUsageEntry(
    Guid WorkspaceId,
    Guid UserId,
    string FeatureId,
    string PromptVersion,
    string Audience,
    string Provider,
    string? Model,
    string Outcome)
{
    public Guid? SubjectUserId { get; init; }
    public Guid? ProjectId { get; init; }
    public DateTime? RangeFrom { get; init; }
    public DateTime? RangeTo { get; init; }
    public IReadOnlyCollection<string> DataCategories { get; init; } = Array.Empty<string>();
    public bool CacheHit { get; init; }
    public bool IsRegenerate { get; init; }
    public string? ErrorCode { get; init; }
    public AiTokenUsage Usage { get; init; } = AiTokenUsage.None;
    public decimal? EstimatedCostUsd { get; init; }
    public int? LatencyMs { get; init; }
    public string? ToolCalls { get; init; }
}

/// <summary>
/// Writes the usage ledger (<see cref="AiUsageRecord"/>). Each row is written in a short unit of work of its own, never
/// cancelled: a request whose caller went away still counts, and a failed request after it does not undo it.
/// </summary>
public class AiUsageRecorder : ITransientDependency
{
    private readonly IUnitOfWorkManager _unitOfWorkManager;
    private readonly IRepository<AiUsageRecord, Guid> _repository;
    private readonly IGuidGenerator _guidGenerator;

    public AiUsageRecorder(IUnitOfWorkManager unitOfWorkManager, IRepository<AiUsageRecord, Guid> repository, IGuidGenerator guidGenerator)
    {
        _unitOfWorkManager = unitOfWorkManager;
        _repository = repository;
        _guidGenerator = guidGenerator;
    }

    /// <summary>Writes the row and returns its id (the result id callers rate with feedback).</summary>
    public virtual async Task<Guid> RecordAsync(AiUsageEntry entry)
    {
        var record = new AiUsageRecord(
            _guidGenerator.Create(),
            entry.WorkspaceId,
            entry.UserId,
            entry.FeatureId,
            entry.PromptVersion,
            entry.Audience,
            entry.Provider,
            entry.Model,
            entry.Outcome,
            string.Join(",", entry.DataCategories.Distinct(StringComparer.Ordinal)),
            entry.SubjectUserId,
            entry.ProjectId,
            entry.RangeFrom,
            entry.RangeTo,
            entry.CacheHit,
            entry.IsRegenerate);
        record.SetOutcome(entry.Outcome, entry.ErrorCode);
        record.SetUsage(
            entry.Usage.InputTokens,
            entry.Usage.OutputTokens,
            entry.Usage.CachedInputTokens,
            entry.Usage.ReasoningTokens,
            entry.EstimatedCostUsd,
            entry.LatencyMs,
            entry.ToolCalls);

        using var uow = _unitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
        await _repository.InsertAsync(record, cancellationToken: CancellationToken.None);
        await uow.CompleteAsync(CancellationToken.None);
        return record.Id;
    }

    /// <summary>Rates a result. Only the member who received it may rate it; anyone else's id is answered as missing.</summary>
    /// <param name="rating"><see cref="AiUsageRecord.FeedbackUp"/> or <see cref="AiUsageRecord.FeedbackDown"/>.</param>
    /// <exception cref="EntityNotFoundException">No such result in this workspace, or it belongs to someone else.</exception>
    public virtual async Task SetFeedbackAsync(Guid resultId, Guid userId, string rating, string? reason)
    {
        var record = await _repository.FindAsync(resultId);
        if (record == null || record.UserId != userId)
        {
            throw new EntityNotFoundException(typeof(AiUsageRecord), resultId);
        }

        record.SetFeedback(rating, reason);
        await _repository.UpdateAsync(record);
    }
}
