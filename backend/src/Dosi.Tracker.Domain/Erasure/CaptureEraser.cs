using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Microsoft.Extensions.Logging;
using Volo.Abp.BlobStoring;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Erasure;

/// <summary>What <see cref="CaptureEraser.DeleteAsync"/> did.</summary>
public sealed record CaptureErasureResult(int Deleted, int MissingFiles, int FailedFiles);

/// <summary>
/// Deletes captures for good: each file in the blob store first, then the rows (saved with the caller's unit of work).
/// A file that is already gone is fine. A file whose deletion fails keeps its row, so the capture can be deleted again
/// later, unless the caller asked for the failure to be thrown. A row whose blob key does not follow the writer's
/// convention (<see cref="CaptureRetentionService.IsCanonicalStorageUrl"/>) loses its row but its key is never handed
/// to the store: a corrupt key must not delete somebody else's file.
/// </summary>
public class CaptureEraser : TrackerDomainService
{
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IBlobContainer _blobContainer;

    public CaptureEraser(IRepository<Screenshot, Guid> screenshotRepository, IBlobContainer blobContainer)
    {
        _screenshotRepository = screenshotRepository;
        _blobContainer = blobContainer;
    }

    public virtual async Task<CaptureErasureResult> DeleteAsync(IReadOnlyCollection<Screenshot> captures, bool throwOnFileFailure)
    {
        var deletable = new List<Screenshot>(captures.Count);
        var missing = 0;
        var failed = 0;
        foreach (var capture in captures)
        {
            if (!CaptureRetentionService.IsCanonicalStorageUrl(capture.StorageUrl, capture.ActivityId, capture.Kind))
            {
                Logger.LogWarning(
                    "Capture {CaptureId} has the non-canonical blob key '{StorageUrl}'; its row is deleted but the key is not touched.",
                    capture.Id, capture.StorageUrl);
                deletable.Add(capture);
                continue;
            }

            try
            {
                if (!await _blobContainer.DeleteAsync(capture.StorageUrl))
                {
                    missing++;
                }

                deletable.Add(capture);
            }
            catch (Exception exception) when (CaptureRetentionService.IsBlobNotFoundException(exception))
            {
                missing++;
                deletable.Add(capture);
            }
            catch (Exception exception) when (!throwOnFileFailure)
            {
                failed++;
                Logger.LogWarning(exception,
                    "Could not delete the file of capture {CaptureId}; its row is kept so that it is deleted later.",
                    capture.Id);
            }
        }

        if (deletable.Count > 0)
        {
            await _screenshotRepository.DeleteManyAsync(deletable, autoSave: true);
        }

        return new CaptureErasureResult(deletable.Count, missing, failed);
    }
}
