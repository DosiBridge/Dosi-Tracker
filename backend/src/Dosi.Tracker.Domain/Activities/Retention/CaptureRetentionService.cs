using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Volo.Abp.BlobStoring;
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
/// Production service implementing workspace-scoped capture retention (REQ-DAT-01).
/// Guarantees:
/// 1. Explicit workspace scoping with explicit WorkspaceId predicates in selection and mutation.
/// 2. Bounded batch pagination with deterministic ordering (CapturedAt, Id).
/// 3. Preservation of records at or newer than cutoff (CapturedAt >= cutoff) and unrelated workspaces.
/// 4. Canonical blob key validation preventing arbitrary deletion of caller-supplied or external paths.
/// 5. Non-atomic distributed boundary handling: blob deletion failure preserves DB record for retry;
///    missing blobs are safely reclaimed; DB failures after blob deletion converge on retry.
/// 6. Scoped tenant filter control without global leakage.
/// </summary>
public class CaptureRetentionService : ICaptureRetentionService, ITransientDependency
{
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IBlobContainer _blobContainer;
    private readonly IUnitOfWorkManager _unitOfWorkManager;
    private readonly IDataFilter _dataFilter;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly IClock _clock;
    private readonly ILogger<CaptureRetentionService> _logger;

    public CaptureRetentionService(
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<Workspace, Guid> workspaceRepository,
        IBlobContainer blobContainer,
        IUnitOfWorkManager unitOfWorkManager,
        IDataFilter dataFilter,
        IAsyncQueryableExecuter asyncExecuter,
        IClock clock,
        ILogger<CaptureRetentionService> logger)
    {
        _screenshotRepository = screenshotRepository;
        _workspaceRepository = workspaceRepository;
        _blobContainer = blobContainer;
        _unitOfWorkManager = unitOfWorkManager;
        _dataFilter = dataFilter;
        _asyncExecuter = asyncExecuter;
        _clock = clock;
        _logger = logger;
    }

    public async Task<CaptureRetentionResult> CleanExpiredCapturesAsync(
        CaptureRetentionPolicy policy,
        CancellationToken cancellationToken = default)
    {
        if (policy == null)
        {
            throw new ArgumentNullException(nameof(policy));
        }

        // Validate policy before touching any resources
        policy.Validate();

        var cutoff = policy.Cutoff ?? (_clock.Now - policy.RetentionPeriod);

        // Read phase: verify workspace and query candidates under a single read Unit of Work
        List<Screenshot> candidateList;
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
                    "Workspace {WorkspaceId} is archived and policy.AllowArchivedWorkspace is false. Skipping retention.",
                    policy.WorkspaceId);

                return new CaptureRetentionResult
                {
                    WorkspaceId = policy.WorkspaceId,
                    Cutoff = cutoff,
                    CandidateCount = 0,
                    DeletedCount = 0,
                    FailedBlobCount = 0,
                    MissingBlobCount = 0,
                    CorruptKeyCount = 0,
                    HasMore = false,
                    Skipped = true,
                    SkipReason = "Workspace is archived"
                };
            }

            var query = (await _screenshotRepository.GetQueryableAsync())
                .Where(s => s.WorkspaceId == policy.WorkspaceId && s.CapturedAt < cutoff);

            if (policy.Kinds != null && policy.Kinds.Count > 0)
            {
                query = query.Where(s => policy.Kinds.Contains(s.Kind));
            }

            query = query
                .OrderBy(s => s.CapturedAt)
                .ThenBy(s => s.Id)
                .Take(policy.BatchSize + 1);

            candidateList = await _asyncExecuter.ToListAsync(query, cancellationToken);
            await readUow.CompleteAsync(cancellationToken);
        }

        var hasMore = candidateList.Count > policy.BatchSize;
        var batch = candidateList.Take(policy.BatchSize).ToList();

        if (batch.Count == 0)
        {
            return new CaptureRetentionResult
            {
                WorkspaceId = policy.WorkspaceId,
                Cutoff = cutoff,
                CandidateCount = 0,
                DeletedCount = 0,
                FailedBlobCount = 0,
                MissingBlobCount = 0,
                CorruptKeyCount = 0,
                HasMore = false,
                Skipped = false
            };
        }

        var toDeleteFromDb = new List<Screenshot>(batch.Count);
        var failedBlobCount = 0;
        var missingBlobCount = 0;
        var corruptKeyCount = 0;

        foreach (var screenshot in batch)
        {
            cancellationToken.ThrowIfCancellationRequested();

            // Enforce canonical blob ownership / key convention
            if (!IsCanonicalStorageUrl(screenshot.StorageUrl, screenshot.ActivityId, screenshot.Kind))
            {
                _logger.LogWarning(
                    "Screenshot {ScreenshotId} in workspace {WorkspaceId} has non-canonical StorageUrl '{StorageUrl}'. Skipping blob deletion to prevent arbitrary key deletion.",
                    screenshot.Id, screenshot.WorkspaceId, screenshot.StorageUrl);
                corruptKeyCount++;
                continue;
            }

            var safeForDbDelete = false;
            try
            {
                var existed = await _blobContainer.DeleteAsync(screenshot.StorageUrl, cancellationToken);
                if (!existed)
                {
                    // Blob was already missing from storage; safe to clean up DB record
                    missingBlobCount++;
                }
                safeForDbDelete = true;
            }
            catch (Exception ex) when (IsBlobNotFoundException(ex))
            {
                // Provider indicated blob not found; safe to clean up DB record
                missingBlobCount++;
                safeForDbDelete = true;
            }
            catch (Exception ex)
            {
                // Real storage deletion failure (e.g. S3 timeout, auth error):
                // DO NOT delete DB row; preserve pointer so retry can re-attempt deletion.
                _logger.LogWarning(ex,
                    "Failed to delete blob '{StorageUrl}' for screenshot {ScreenshotId}. DB pointer preserved for retry.",
                    screenshot.StorageUrl, screenshot.Id);
                failedBlobCount++;
                safeForDbDelete = false;
            }

            if (safeForDbDelete)
            {
                toDeleteFromDb.Add(screenshot);
            }
        }

        // Commit database deletions under transactional UOW with explicit WorkspaceId predicate
        var deletedCount = 0;
        if (toDeleteFromDb.Count > 0)
        {
            using var uow = _unitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
            using (_dataFilter.Disable<IWorkspaceScoped>())
            {
                var toDeleteIds = toDeleteFromDb.Select(s => s.Id).ToList();

                // Explicit WorkspaceId predicate defense-in-depth against accidental cross-tenant deletion
                var entitiesToDelete = await _screenshotRepository.GetListAsync(
                    s => toDeleteIds.Contains(s.Id) && s.WorkspaceId == policy.WorkspaceId,
                    cancellationToken: cancellationToken);

                await _screenshotRepository.DeleteManyAsync(entitiesToDelete, autoSave: true, cancellationToken: cancellationToken);
                deletedCount = entitiesToDelete.Count;
            }

            await uow.CompleteAsync(cancellationToken);
        }

        return new CaptureRetentionResult
        {
            WorkspaceId = policy.WorkspaceId,
            Cutoff = cutoff,
            CandidateCount = batch.Count,
            DeletedCount = deletedCount,
            FailedBlobCount = failedBlobCount,
            MissingBlobCount = missingBlobCount,
            CorruptKeyCount = corruptKeyCount,
            HasMore = hasMore,
            Skipped = false
        };
    }

    /// <summary>
    /// Canonical blob key convention: activities/{activityId}/{kind}{extension}
    /// Rejects arbitrary URLs, filesystem paths, path traversal, or mismatched activity/kind bindings.
    /// </summary>
    public static bool IsCanonicalStorageUrl(string? storageUrl, Guid activityId, string? kind)
    {
        if (string.IsNullOrWhiteSpace(storageUrl) || string.IsNullOrWhiteSpace(kind) || activityId == Guid.Empty)
        {
            return false;
        }

        var expectedPrefix = $"activities/{activityId}/";
        if (!storageUrl.StartsWith(expectedPrefix, StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        // Prohibit path traversal or directory manipulation
        if (storageUrl.Contains("..") || storageUrl.Contains('\\'))
        {
            return false;
        }

        var fileName = storageUrl.Substring(expectedPrefix.Length);
        if (!fileName.StartsWith(kind, StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        return true;
    }

    private static bool IsBlobNotFoundException(Exception ex)
    {
        return ex.GetType().Name.Contains("NotFound", StringComparison.OrdinalIgnoreCase) ||
               ex is KeyNotFoundException;
    }
}
