using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Production domain service implementing workspace-scoped activity and time summary age retention (REQ-DAT-01).
/// Dormant unless explicitly invoked.
/// Guarantees:
/// 1. Explicit workspace scoping with explicit WorkspaceId predicates in selection and mutation.
/// 2. Bounded batch pagination with deterministic ordering (StartedAt, Id) and exact-cutoff preservation.
/// 3. Preservation of activities still referenced by capture metadata (preventing cascade deletion or orphaned blobs).
/// 4. Preservation of activities in archived workspaces by default (unless AllowArchivedWorkspace is explicitly enabled).
/// 5. Bounded keyset cursor advancement past preserved dependencies, preventing infinite pagination loops.
/// 6. Zero interaction with blob storage (does not touch, orphan, or delete blobs).
/// </summary>
public class ActivityRetentionService : IActivityRetentionService, ITransientDependency
{
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IUnitOfWorkManager _unitOfWorkManager;
    private readonly IDataFilter _dataFilter;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly IClock _clock;
    private readonly ILogger<ActivityRetentionService> _logger;

    public ActivityRetentionService(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<Workspace, Guid> workspaceRepository,
        IUnitOfWorkManager unitOfWorkManager,
        IDataFilter dataFilter,
        IAsyncQueryableExecuter asyncExecuter,
        IClock clock,
        ILogger<ActivityRetentionService> logger)
    {
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _workspaceRepository = workspaceRepository;
        _unitOfWorkManager = unitOfWorkManager;
        _dataFilter = dataFilter;
        _asyncExecuter = asyncExecuter;
        _clock = clock;
        _logger = logger;
    }

    public async Task<ActivityRetentionResult> CleanExpiredActivitiesAsync(
        ActivityRetentionPolicy policy,
        CancellationToken cancellationToken = default)
    {
        if (policy == null)
        {
            throw new ArgumentNullException(nameof(policy));
        }

        // Validate policy before touching any resources
        policy.Validate();

        var cutoff = policy.Cutoff ?? (_clock.Now - policy.RetentionPeriod);

        List<Activity> candidateList;
        HashSet<Guid> referencedActivityIds;

        // Read phase: verify workspace and query candidates under a read Unit of Work
        using (var readUow = _unitOfWorkManager.Begin(requiresNew: true))
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            var workspace = await _workspaceRepository.FindAsync(policy.WorkspaceId, cancellationToken: cancellationToken);

            if (workspace == null)
            {
                throw new EntityNotFoundException(typeof(Workspace), policy.WorkspaceId);
            }

            if (workspace.IsArchived && !policy.AllowArchivedWorkspace)
            {
                _logger.LogInformation(
                    "Workspace {WorkspaceId} is archived and policy.AllowArchivedWorkspace is false. Skipping activity retention.",
                    policy.WorkspaceId);

                return new ActivityRetentionResult
                {
                    WorkspaceId = policy.WorkspaceId,
                    Cutoff = cutoff,
                    CandidateCount = 0,
                    DeletedCount = 0,
                    PreservedReferencedCount = 0,
                    HasMore = false,
                    Skipped = true,
                    SkipReason = "Workspace is archived"
                };
            }

            // Exact-cutoff preservation: activities with StartedAt >= cutoff survive; only StartedAt < cutoff are eligible.
            var query = (await _activityRepository.GetQueryableAsync())
                .Where(a => a.WorkspaceId == policy.WorkspaceId && a.StartedAt < cutoff);

            if (policy.Cursor != null)
            {
                var cursorStartedAt = policy.Cursor.StartedAt;
                var cursorId = policy.Cursor.ActivityId;
                query = query.Where(a => a.StartedAt > cursorStartedAt ||
                    (a.StartedAt == cursorStartedAt && a.Id.CompareTo(cursorId) > 0));
            }

            query = query
                .OrderBy(a => a.StartedAt)
                .ThenBy(a => a.Id)
                .Take(policy.BatchSize + 1);

            candidateList = await _asyncExecuter.ToListAsync(query, cancellationToken);

            var batchCandidateIds = candidateList.Take(policy.BatchSize).Select(a => a.Id).Distinct().ToList();
            if (batchCandidateIds.Count > 0)
            {
                // Dependency check: query screenshots across ALL workspaces (including corrupt/foreign references)
                // that point to any candidate in this batch.
                var refList = await _asyncExecuter.ToListAsync(
                    (await _screenshotRepository.GetQueryableAsync())
                        .Where(s => batchCandidateIds.Contains(s.ActivityId))
                        .Select(s => s.ActivityId)
                        .Distinct(),
                    cancellationToken);

                referencedActivityIds = new HashSet<Guid>(refList);
            }
            else
            {
                referencedActivityIds = new HashSet<Guid>();
            }

            await readUow.CompleteAsync(cancellationToken);
        }

        var hasMore = candidateList.Count > policy.BatchSize;
        var batch = candidateList.Take(policy.BatchSize).ToList();

        if (batch.Count == 0)
        {
            return new ActivityRetentionResult
            {
                WorkspaceId = policy.WorkspaceId,
                Cutoff = cutoff,
                CandidateCount = 0,
                DeletedCount = 0,
                PreservedReferencedCount = 0,
                HasMore = false,
                NextCursor = null,
                LastProcessedCursor = null,
                Skipped = false
            };
        }

        var toDeleteFromDb = new List<Activity>(batch.Count);
        var preservedReferencedCount = 0;

        foreach (var activity in batch)
        {
            cancellationToken.ThrowIfCancellationRequested();

            // Guard: preserve any Activity still referenced by capture metadata (even foreign/corrupt references).
            // Do not cascade delete metadata or orphan blobs; do not call blob provider.
            if (referencedActivityIds.Contains(activity.Id))
            {
                _logger.LogInformation(
                    "Activity {ActivityId} in workspace {WorkspaceId} is still referenced by capture metadata. Preserving activity row.",
                    activity.Id, activity.WorkspaceId);
                preservedReferencedCount++;
                continue;
            }

            // Explicit workspace ownership check on candidate entity
            if (activity.WorkspaceId != policy.WorkspaceId)
            {
                _logger.LogWarning(
                    "Activity {ActivityId} has mismatched WorkspaceId {ActivityWsId} vs policy {PolicyWsId}. Failing closed without deleting.",
                    activity.Id, activity.WorkspaceId, policy.WorkspaceId);
                continue;
            }

            toDeleteFromDb.Add(activity);
        }

        // Commit database deletions under transactional UOW with explicit WorkspaceId predicate
        var deletedCount = 0;
        if (toDeleteFromDb.Count > 0)
        {
            using var uow = _unitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
            using (_dataFilter.Disable<IWorkspaceScoped>())
            {
                var toDeleteIds = toDeleteFromDb.Select(a => a.Id).ToList();

                // Explicit WorkspaceId predicate defense-in-depth against accidental cross-tenant deletion
                var entitiesToDelete = await _activityRepository.GetListAsync(
                    a => toDeleteIds.Contains(a.Id) && a.WorkspaceId == policy.WorkspaceId,
                    cancellationToken: cancellationToken);

                await _activityRepository.DeleteManyAsync(entitiesToDelete, autoSave: true, cancellationToken: cancellationToken);
                deletedCount = entitiesToDelete.Count;
            }

            await uow.CompleteAsync(cancellationToken);
        }

        // Deterministic cursor advancement past evaluated batch, even if some candidates were preserved due to dependencies
        var lastCandidate = batch.Count > 0 ? batch[^1] : null;
        var lastCursor = lastCandidate != null ? new ActivityRetentionCursor(lastCandidate.StartedAt, lastCandidate.Id) : null;
        var nextCursor = hasMore && lastCursor != null ? lastCursor : null;

        return new ActivityRetentionResult
        {
            WorkspaceId = policy.WorkspaceId,
            Cutoff = cutoff,
            CandidateCount = batch.Count,
            DeletedCount = deletedCount,
            PreservedReferencedCount = preservedReferencedCount,
            HasMore = hasMore,
            NextCursor = nextCursor,
            LastProcessedCursor = lastCursor,
            Skipped = false
        };
    }
}
