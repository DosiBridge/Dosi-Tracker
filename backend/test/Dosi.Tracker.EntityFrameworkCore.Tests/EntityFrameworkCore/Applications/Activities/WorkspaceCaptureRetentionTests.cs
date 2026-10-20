using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.BlobStoring;
using Volo.Abp.Data;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-DAT-01: Workspace-isolated capture retention lifecycle test suite.
/// Uses isolated SQLite in-memory database and synthetic in-memory ScriptedBlobProvider.
/// Exercises:
/// 1. A-expired deletion with persisted-row and fake-blob assertions.
/// 2. B-expired sentinel unchanged.
/// 3. A-fresh and exact-cutoff unchanged.
/// 4. Activity and time summaries remain completely untouched.
/// 5. Archived workspace policy explicit (skip default vs allowed).
/// 6. Zero/empty/negative policy refusal before side effects.
/// 7. Bounded batches / repeat convergence.
/// 8. Missing blobs safely handled and DB rows cleaned.
/// 9. Provider failure retaining retriable metadata.
/// 10. Crash/retry convergence after blob deletion.
/// 11. Canonical blob key validation rejecting arbitrary/caller-supplied paths.
/// 12. Default-disabled worker no side effects.
/// 13. Enabled worker sweeping active workspaces and skipping archived.
/// 14. Cross-workspace inconsistent Activity reference fails closed without deleting blob or metadata.
/// 15. Exact-key negative regressions rejecting sibling, nested, unknown kind, case-folded, and unsupported format.
/// 16. Legitimate screen/thumb/webcam kinds and .jpg/.png format variants clean up successfully.
/// </summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspaceCaptureRetentionTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly Guid TestUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly ScriptedBlobProvider _blobProvider = new();

    private readonly ICaptureRetentionService _retentionService;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IClock _clock;
    private readonly IGuidGenerator _guidGenerator;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceCaptureRetentionTests()
    {
        _retentionService = GetRequiredService<ICaptureRetentionService>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _clock = GetRequiredService<IClock>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private async Task<(Guid WorkspaceId, Guid ProjectId, Guid ActivityId)> SeedWorkspaceAndActivityAsync(string name)
    {
        var wsId = await ServiceProvider.CreateWorkspaceAsync(name);
        var projId = _guidGenerator.Create();
        var actId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _projectRepository.InsertAsync(new Project(projId, wsId, $"{name}-Project"), autoSave: true);

                var activity = new Activity(
                    id: actId,
                    workspaceId: wsId,
                    userId: TestUserId,
                    projectId: projId,
                    clientActivityId: _guidGenerator.Create(),
                    startedAt: _clock.Now.AddDays(-50),
                    endedAt: _clock.Now.AddDays(-50).AddMinutes(10))
                {
                    Productivity = 88,
                    MouseClicks = 250,
                    KeyboardHits = 600,
                    ActiveWindowsJson = "[\"Code.exe\"]",
                    RunningProgramsJson = "[\"devenv.exe\"]",
                    TimelineJson = "[{\"m\":0,\"k\":60}]"
                };

                await _activityRepository.InsertAsync(activity, autoSave: true);
            }
        });

        return (wsId, projId, actId);
    }

    private async Task<Guid> SeedActivityInWorkspaceAsync(Guid workspaceId, Guid projectId, DateTime? startedAt = null)
    {
        var actId = _guidGenerator.Create();
        var start = startedAt ?? _clock.Now.AddDays(-50);

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var activity = new Activity(
                    id: actId,
                    workspaceId: workspaceId,
                    userId: TestUserId,
                    projectId: projectId,
                    clientActivityId: _guidGenerator.Create(),
                    startedAt: start,
                    endedAt: start.AddMinutes(10))
                {
                    Productivity = 88,
                    MouseClicks = 250,
                    KeyboardHits = 600,
                    ActiveWindowsJson = "[\"Code.exe\"]",
                    RunningProgramsJson = "[\"devenv.exe\"]",
                    TimelineJson = "[{\"m\":0,\"k\":60}]"
                };

                await _activityRepository.InsertAsync(activity, autoSave: true);
            }
        });

        return actId;
    }

    private async Task<Screenshot> SeedScreenshotAsync(
        Guid workspaceId,
        Guid activityId,
        string kind,
        DateTime capturedAt,
        string? customStorageUrl = null,
        byte[]? syntheticBytes = null,
        string contentType = "image/jpeg")
    {
        var id = _guidGenerator.Create();
        var ext = contentType == "image/png" ? ".png" : ".jpg";
        var storageUrl = customStorageUrl ?? $"activities/{activityId:D}/{kind}{ext}";
        var screenshot = new Screenshot(
            id: id,
            workspaceId: workspaceId,
            activityId: activityId,
            storageUrl: storageUrl,
            capturedAt: capturedAt,
            sizeBytes: syntheticBytes?.Length ?? 1024,
            contentType: contentType,
            kind: kind);

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _screenshotRepository.InsertAsync(screenshot, autoSave: true);
            }
        });

        _blobProvider.Blobs[storageUrl] = syntheticBytes ?? new byte[] { 0xDE, 0xAD, 0xBE, 0xEF };

        return screenshot;
    }

    [Fact]
    public async Task Expired_Captures_Are_Deleted_From_Database_And_Blob_Container()
    {
        // Arrange: Workspace A with 2 expired captures (screen and thumb) older than 30-day retention
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-EXP-A");
        var cutoffTarget = _clock.Now.AddDays(-30);

        var sc1 = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, cutoffTarget.AddDays(-5));
        var sc2 = await SeedScreenshotAsync(wsA, actA, Screenshot.ThumbKind, cutoffTarget.AddDays(-2));

        _blobProvider.Blobs.ContainsKey(sc1.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(sc2.StorageUrl).ShouldBeTrue();

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert
        result.CandidateCount.ShouldBe(2);
        result.DeletedCount.ShouldBe(2);
        result.FailedBlobCount.ShouldBe(0);
        result.MissingBlobCount.ShouldBe(0);
        result.CorruptKeyCount.ShouldBe(0);
        result.Skipped.ShouldBeFalse();

        // Database assertions: persisted rows deleted
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc1.Id)).ShouldBeNull();
            (await _screenshotRepository.FindAsync(sc2.Id)).ShouldBeNull();
        }

        // Fake-blob assertions: in-memory blobs removed
        _blobProvider.Blobs.ContainsKey(sc1.StorageUrl).ShouldBeFalse();
        _blobProvider.Blobs.ContainsKey(sc2.StorageUrl).ShouldBeFalse();
    }

    [Fact]
    public async Task Unrelated_Workspace_Expired_Captures_Are_Preserved_Untouched()
    {
        // Arrange: Workspace A and Workspace B both have expired captures
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-ISO-A");
        var (wsB, _, actB) = await SeedWorkspaceAndActivityAsync("RET-ISO-B");
        var expiredTime = _clock.Now.AddDays(-40);

        var scA = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, expiredTime);
        var scB = await SeedScreenshotAsync(wsB, actB, Screenshot.ScreenKind, expiredTime);

        var policyA = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act: clean only Workspace A
        var result = await _retentionService.CleanExpiredCapturesAsync(policyA);

        // Assert: Workspace A cleaned
        result.DeletedCount.ShouldBe(1);
        _blobProvider.Blobs.ContainsKey(scA.StorageUrl).ShouldBeFalse();

        // Sentinel: Workspace B completely unchanged
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var bInDb = await _screenshotRepository.FindAsync(scB.Id);
            bInDb.ShouldNotBeNull();
            bInDb.WorkspaceId.ShouldBe(wsB);
        }
        _blobProvider.Blobs.ContainsKey(scB.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Fresh_And_Exact_Cutoff_Captures_Are_Preserved_Untouched()
    {
        // Arrange: Workspace A has fresh, exact cutoff, and expired captures
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-BOUNDARY-A");
        var retention = TimeSpan.FromDays(30);
        var deterministicCutoff = _clock.Now - retention;

        var scFresh = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, deterministicCutoff.AddMinutes(10));
        var scExact = await SeedScreenshotAsync(wsA, actA, Screenshot.ThumbKind, deterministicCutoff);
        var scExpired = await SeedScreenshotAsync(wsA, actA, Screenshot.WebcamKind, deterministicCutoff.AddMinutes(-10));

        var policy = new CaptureRetentionPolicy(wsA, retention, cutoff: deterministicCutoff);

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert
        result.CandidateCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(1);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scFresh.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(scExact.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(scExpired.Id)).ShouldBeNull();
        }

        _blobProvider.Blobs.ContainsKey(scFresh.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(scExact.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(scExpired.StorageUrl).ShouldBeFalse();
    }

    [Fact]
    public async Task Activity_Time_Summaries_And_Blocks_Remain_Completely_Untouched()
    {
        // Arrange: Activity with metrics and child expired screenshot
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-ACTIVITY-A");
        var expiredTime = _clock.Now.AddDays(-45);
        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, expiredTime);

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: Screenshot was deleted
        result.DeletedCount.ShouldBe(1);

        // Parent Activity, time blocks, productivity, and input metrics remain 100% intact
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var activity = await _activityRepository.FindAsync(actA);
            activity.ShouldNotBeNull();
            activity.WorkspaceId.ShouldBe(wsA);
            activity.Productivity.ShouldBe(88);
            activity.MouseClicks.ShouldBe(250);
            activity.KeyboardHits.ShouldBe(600);
            activity.ActiveWindowsJson.ShouldBe("[\"Code.exe\"]");
            activity.RunningProgramsJson.ShouldBe("[\"devenv.exe\"]");
            activity.TimelineJson.ShouldBe("[{\"m\":0,\"k\":60}]");
        }
    }

    [Fact]
    public async Task Archived_Workspace_Policy_Explicit_Default_Skips_Unless_Explicitly_Allowed()
    {
        // Arrange: Archived workspace with expired capture
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-ARCHIVED-A");
        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var ws = await _workspaceRepository.GetAsync(wsA);
                ws.Archive(TestUserId, _clock.Now);
                await _workspaceRepository.UpdateAsync(ws, autoSave: true);
            }
        });

        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        // Case 1: Default policy (AllowArchivedWorkspace = false) skips
        var defaultPolicy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30), allowArchivedWorkspace: false);
        var resultSkip = await _retentionService.CleanExpiredCapturesAsync(defaultPolicy);

        resultSkip.Skipped.ShouldBeTrue();
        resultSkip.SkipReason.ShouldBe("Workspace is archived");
        resultSkip.DeletedCount.ShouldBe(0);

        // Both DB row and fake blob preserved
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeTrue();

        // Case 2: Explicitly allowed policy executes retention
        var allowPolicy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30), allowArchivedWorkspace: true);
        var resultAllow = await _retentionService.CleanExpiredCapturesAsync(allowPolicy);

        resultAllow.Skipped.ShouldBeFalse();
        resultAllow.DeletedCount.ShouldBe(1);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeFalse();
    }

    [Fact]
    public async Task Zero_Empty_Or_Negative_Policy_Throws_Before_Any_Side_Effects()
    {
        var validWs = Guid.NewGuid();

        // Guid.Empty throws ArgumentException
        Should.Throw<ArgumentException>(() =>
            new CaptureRetentionPolicy(Guid.Empty, TimeSpan.FromDays(30)).Validate());

        // TimeSpan.Zero throws ArgumentOutOfRangeException
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new CaptureRetentionPolicy(validWs, TimeSpan.Zero).Validate());

        // Negative TimeSpan throws ArgumentOutOfRangeException
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new CaptureRetentionPolicy(validWs, TimeSpan.FromDays(-5)).Validate());

        // Zero batch size throws ArgumentOutOfRangeException
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new CaptureRetentionPolicy(validWs, TimeSpan.FromDays(30), batchSize: 0).Validate());

        // Negative batch size throws ArgumentOutOfRangeException
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new CaptureRetentionPolicy(validWs, TimeSpan.FromDays(30), batchSize: -1).Validate());

        // Excessive batch size (> 1000) throws ArgumentOutOfRangeException
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new CaptureRetentionPolicy(validWs, TimeSpan.FromDays(30), batchSize: 1001).Validate());

        // Null policy passed to service throws ArgumentNullException
        await Should.ThrowAsync<ArgumentNullException>(() =>
            _retentionService.CleanExpiredCapturesAsync(null!));
    }

    [Fact]
    public async Task Bounded_Batching_And_HasMore_Convergence_On_Repeat_Execution()
    {
        // Arrange: 5 expired screenshots across distinct synthetic activities in Workspace A with batch size = 2
        var (wsA, projA, actA) = await SeedWorkspaceAndActivityAsync("RET-BATCH-A");
        var baseTime = _clock.Now.AddDays(-50);

        var list = new List<Screenshot>
        {
            await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, baseTime)
        };

        for (int i = 1; i < 5; i++)
        {
            var nextActId = await SeedActivityInWorkspaceAsync(wsA, projA, startedAt: baseTime.AddMinutes(i * 15));
            var sc = await SeedScreenshotAsync(wsA, nextActId, Screenshot.ScreenKind, baseTime.AddMinutes(i * 15 + 1));
            list.Add(sc);
        }

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30), batchSize: 2);

        // Act & Assert Pass 1: processes 2, hasMore = true
        var pass1 = await _retentionService.CleanExpiredCapturesAsync(policy);
        pass1.CandidateCount.ShouldBe(2);
        pass1.DeletedCount.ShouldBe(2);
        pass1.HasMore.ShouldBeTrue();

        // Act & Assert Pass 2: processes 2, hasMore = true
        var pass2 = await _retentionService.CleanExpiredCapturesAsync(policy);
        pass2.CandidateCount.ShouldBe(2);
        pass2.DeletedCount.ShouldBe(2);
        pass2.HasMore.ShouldBeTrue();

        // Act & Assert Pass 3: processes remaining 1, hasMore = false
        var pass3 = await _retentionService.CleanExpiredCapturesAsync(policy);
        pass3.CandidateCount.ShouldBe(1);
        pass3.DeletedCount.ShouldBe(1);
        pass3.HasMore.ShouldBeFalse();

        // Act & Assert Pass 4: idempotent subsequent run produces 0 deletions
        var pass4 = await _retentionService.CleanExpiredCapturesAsync(policy);
        pass4.CandidateCount.ShouldBe(0);
        pass4.DeletedCount.ShouldBe(0);
        pass4.HasMore.ShouldBeFalse();

        // All 5 blobs removed
        foreach (var sc in list)
        {
            _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeFalse();
        }
    }

    [Fact]
    public async Task Missing_Blobs_In_Storage_Safely_Clean_Up_Dangling_Database_Rows()
    {
        // Arrange: Screenshot row exists in DB, but blob is missing from storage (e.g. manual delete or lifecycle)
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-MISSING-A");
        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        // Delete blob beforehand from storage to simulate missing object
        _blobProvider.Blobs.TryRemove(sc.StorageUrl, out _);
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeFalse();

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: safely increments MissingBlobCount and cleans dangling DB row
        result.CandidateCount.ShouldBe(1);
        result.MissingBlobCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(1);
        result.FailedBlobCount.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldBeNull();
        }
    }

    [Fact]
    public async Task Blob_Deletion_Failure_Preserves_Database_Pointer_For_Retry()
    {
        // Arrange: Storage failure (e.g., S3 503) injected into blob provider
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-FAIL-A");
        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        _blobProvider.BeforeDelete = (num, key) =>
            throw new HttpRequestException("S3 503 Service Unavailable");

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act: Run with failing storage
        var resultFail = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: DB row was NOT deleted; retriable metadata preserved
        resultFail.FailedBlobCount.ShouldBe(1);
        resultFail.DeletedCount.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeTrue();

        // Act: Clear storage failure and retry
        _blobProvider.BeforeDelete = null;
        var resultRetry = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: Successfully deleted on retry
        resultRetry.FailedBlobCount.ShouldBe(0);
        resultRetry.DeletedCount.ShouldBe(1);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeFalse();
    }

    /// <summary>
    /// Note: Manual removal of a fake blob in this test is modeled missing-blob recovery,
    /// NOT an injected real DB commit failure.
    /// Scenario: Blob was deleted in a previous execution before a crash prevented DB commit.
    /// On retry, the blob is already missing, so the service safely cleans the DB row.
    /// </summary>
    [Fact]
    public async Task Database_Failure_After_Blob_Deletion_Converges_Honestly_On_Retry()
    {
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-CRASH-A");
        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        // Manually simulate post-blob-deletion state (blob gone from storage, row still in DB)
        _blobProvider.Blobs.TryRemove(sc.StorageUrl, out _);

        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act: retry execution
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: honest convergence
        result.MissingBlobCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(1);
        result.FailedBlobCount.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldBeNull();
        }
    }

    [Fact]
    public async Task Corrupt_Or_Non_Canonical_Storage_Urls_Are_Rejected_Without_Deleting_Arbitrary_Blobs()
    {
        // Arrange: Workspace A with screenshots holding non-canonical or traversal keys
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-CANONICAL-A");
        var expiredTime = _clock.Now.AddDays(-40);

        var traversalUrl = $"activities/{actA}/../../etc/passwd";
        var mismatchedKindUrl = $"activities/{actA}/unrelated.png";
        var externalUrl = "https://external-storage.com/bucket/secret.jpg";

        var sc1 = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, expiredTime, customStorageUrl: traversalUrl);
        var sc2 = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, expiredTime, customStorageUrl: mismatchedKindUrl);
        var sc3 = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, expiredTime, customStorageUrl: externalUrl);

        var initialDeleteCount = _blobProvider.DeleteCount;
        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: rejected by canonical validator; 0 deletions, blobs untouched
        result.CorruptKeyCount.ShouldBe(3);
        result.DeletedCount.ShouldBe(0);
        (_blobProvider.DeleteCount - initialDeleteCount).ShouldBe(0);

        _blobProvider.Blobs.ContainsKey(traversalUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(mismatchedKindUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(externalUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Worker_Default_Disabled_Performs_Zero_Work_And_Produces_No_Side_Effects()
    {
        // Arrange: Workspace A with expired capture
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-WORKER-DISABLED-A");
        var sc = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        // Default options: IsEnabled = false
        var defaultOptions = Options.Create(new CaptureRetentionOptions { IsEnabled = false });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();

        var worker = new CaptureRetentionWorker(timer, scopeFactory, defaultOptions);

        // Act
        var result = await worker.SweepAsync(ServiceProvider);

        // Assert: Skipped by configuration; 0 side effects
        result.Skipped.ShouldBeTrue();
        result.SkipReason.ShouldBe("Disabled by configuration");
        result.WorkspacesSwept.ShouldBe(0);
        result.TotalDeleted.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(sc.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Worker_Enabled_Sweeps_Active_Workspaces_And_Skips_Archived()
    {
        // Arrange: Active Workspace A and Archived Workspace B, both with expired captures
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-WORKER-ACTIVE-A");
        var (wsB, _, actB) = await SeedWorkspaceAndActivityAsync("RET-WORKER-ARCHIVED-B");

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var ws = await _workspaceRepository.GetAsync(wsB);
                ws.Archive(TestUserId, _clock.Now);
                await _workspaceRepository.UpdateAsync(ws, autoSave: true);
            }
        });

        var scA = await SeedScreenshotAsync(wsA, actA, Screenshot.ScreenKind, _clock.Now.AddDays(-40));
        var scB = await SeedScreenshotAsync(wsB, actB, Screenshot.ScreenKind, _clock.Now.AddDays(-40));

        var enabledOptions = Options.Create(new CaptureRetentionOptions
        {
            IsEnabled = true,
            DefaultRetentionDays = 30,
            BatchSize = 100
        });

        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, enabledOptions);

        // Act
        var result = await worker.SweepAsync(ServiceProvider);

        // Assert: Swept active workspaces, skipped archived
        result.Skipped.ShouldBeFalse();
        result.WorkspacesSwept.ShouldBeGreaterThanOrEqualTo(1);

        // Active Workspace A capture was cleaned
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scA.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scA.StorageUrl).ShouldBeFalse();

        // Archived Workspace B capture was skipped by worker sweep
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scB.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scB.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Cross_Workspace_Inconsistent_Activity_Reference_Fails_Closed_Without_Deleting_Blob_Or_Metadata()
    {
        // Arrange:
        // Genuine Workspace B with its own activity, capture, and blob.
        var (wsB, _, actB) = await SeedWorkspaceAndActivityAsync("RET-INCONSISTENT-B");
        var expiredTime = _clock.Now.AddDays(-40);
        var scB = await SeedScreenshotAsync(wsB, actB, Screenshot.ScreenKind, expiredTime);

        // Genuine Workspace A with its own activity.
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-INCONSISTENT-A");

        // Corrupt row in Workspace A: Screenshot.WorkspaceId = wsA, but references actB and actB's canonical blob key!
        // Note: EF Core schema allows this because FK_AppScreenshots_AppActivities_ActivityId only enforces that
        // actB exists in AppActivities, with no composite constraint on (WorkspaceId, ActivityId).
        var corruptUrl = $"activities/{actB:D}/screen.jpg";
        var corruptScreenshot = await SeedScreenshotAsync(
            wsA, actB, Screenshot.ScreenKind, expiredTime, customStorageUrl: corruptUrl);

        var initialDeleteCount = _blobProvider.DeleteCount;
        var policyA = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act: Run retention for Workspace A
        var result = await _retentionService.CleanExpiredCapturesAsync(policyA);

        // Assert:
        // 1. Service fails closed: identifies corrupt ownership, rejects mutation
        result.CorruptKeyCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(0);

        // 2. Zero blob provider delete invocations occurred
        (_blobProvider.DeleteCount - initialDeleteCount).ShouldBe(0);

        // 3. Workspace B's blob remains intact in storage
        _blobProvider.Blobs.ContainsKey(scB.StorageUrl).ShouldBeTrue();

        // 4. Workspace B's DB row and Activity remain intact
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var bInDb = await _screenshotRepository.FindAsync(scB.Id);
            bInDb.ShouldNotBeNull();
            bInDb.WorkspaceId.ShouldBe(wsB);
            bInDb.ActivityId.ShouldBe(actB);

            var actBInDb = await _activityRepository.FindAsync(actB);
            actBInDb.ShouldNotBeNull();
            actBInDb.WorkspaceId.ShouldBe(wsB);

            // 5. Corrupt Workspace A metadata row is NOT deleted (fail closed without deleting metadata)
            var corruptInDb = await _screenshotRepository.FindAsync(corruptScreenshot.Id);
            corruptInDb.ShouldNotBeNull();
            corruptInDb.WorkspaceId.ShouldBe(wsA);
        }
    }

    [Fact]
    public async Task Exact_Key_Negative_Regressions_Reject_Sibling_Nested_UnknownKind_CaseFolded_And_UnsupportedFormat()
    {
        var (wsA, _, actA) = await SeedWorkspaceAndActivityAsync("RET-EXACT-KEYS-A");
        var expiredTime = _clock.Now.AddDays(-40);

        // Unit-level checks on CaptureRetentionService.IsCanonicalStorageUrl
        // 1. Sibling suffix (e.g. screen-other.jpg, screen_0.jpg)
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen-other.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen_0.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();

        // 2. Nested directory segments
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen/nested.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/nested/screen.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();

        // 3. Unknown capture kind
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/audio.jpg", actA, "audio").ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/video.mp4", actA, "video").ShouldBeFalse();

        // 4. Case folded prefix or kind or extension
        CaptureRetentionService.IsCanonicalStorageUrl($"Activities/{actA:D}/screen.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"ACTIVITIES/{actA:D}/screen.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/SCREEN.jpg", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen.JPG", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen.PNG", actA, Screenshot.ScreenKind).ShouldBeFalse();

        // 5. Unsupported format
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen.bmp", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen.webp", actA, Screenshot.ScreenKind).ShouldBeFalse();
        CaptureRetentionService.IsCanonicalStorageUrl($"activities/{actA:D}/screen.gif", actA, Screenshot.ScreenKind).ShouldBeFalse();

        // Service-level execution: seed these non-canonical rows into DB and assert persisted rows and provider operations
        var badUrls = new[]
        {
            $"activities/{actA:D}/screen-other.jpg",
            $"activities/{actA:D}/screen/nested.jpg",
            $"activities/{actA:D}/audio.jpg",
            $"Activities/{actA:D}/screen.jpg",
            $"activities/{actA:D}/screen.webp"
        };

        var badScreenshots = new List<Screenshot>();
        foreach (var url in badUrls)
        {
            var kind = url.Contains("audio") ? "audio" : Screenshot.ScreenKind;
            badScreenshots.Add(await SeedScreenshotAsync(wsA, actA, kind, expiredTime, customStorageUrl: url));
        }

        var initialDeleteCount = _blobProvider.DeleteCount;
        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: all rejected, 0 deleted
        result.CorruptKeyCount.ShouldBe(badUrls.Length);
        result.DeletedCount.ShouldBe(0);
        (_blobProvider.DeleteCount - initialDeleteCount).ShouldBe(0);

        // Blobs and DB rows remain completely untouched
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            foreach (var sc in badScreenshots)
            {
                (await _screenshotRepository.FindAsync(sc.Id)).ShouldNotBeNull();
                _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeTrue();
            }
        }
    }

    [Fact]
    public async Task Legitimate_Capture_Kinds_And_Format_Variants_Clean_Up_Successfully()
    {
        // Tests all legitimate combinations supported by the writer:
        // Kinds: "screen", "thumb", "webcam"
        // Extensions: ".jpg" (image/jpeg), ".png" (image/png)
        var (wsA, projA, actA) = await SeedWorkspaceAndActivityAsync("RET-LEGIT-A");
        var expiredTime = _clock.Now.AddDays(-40);

        var variants = new (string Kind, string ContentType, string Ext)[]
        {
            (Screenshot.ScreenKind, "image/jpeg", ".jpg"),
            (Screenshot.ScreenKind, "image/png", ".png"),
            (Screenshot.ThumbKind, "image/jpeg", ".jpg"),
            (Screenshot.ThumbKind, "image/png", ".png"),
            (Screenshot.WebcamKind, "image/jpeg", ".jpg"),
            (Screenshot.WebcamKind, "image/png", ".png")
        };

        var seededScreenshots = new List<Screenshot>();
        for (int i = 0; i < variants.Length; i++)
        {
            var variant = variants[i];
            // Each combination on a distinct synthetic activity
            var actId = i == 0 ? actA : await SeedActivityInWorkspaceAsync(wsA, projA, startedAt: expiredTime.AddHours(-i));
            var canonicalUrl = $"activities/{actId:D}/{variant.Kind}{variant.Ext}";

            // Verify helper boolean first
            CaptureRetentionService.IsCanonicalStorageUrl(canonicalUrl, actId, variant.Kind).ShouldBeTrue();

            var sc = await SeedScreenshotAsync(
                wsA, actId, variant.Kind, expiredTime.AddMinutes(-i),
                customStorageUrl: canonicalUrl,
                contentType: variant.ContentType);
            seededScreenshots.Add(sc);
        }

        var initialDeleteCount = _blobProvider.DeleteCount;
        var policy = new CaptureRetentionPolicy(wsA, TimeSpan.FromDays(30));

        // Act
        var result = await _retentionService.CleanExpiredCapturesAsync(policy);

        // Assert: all 6 cleaned cleanly
        result.CandidateCount.ShouldBe(variants.Length);
        result.DeletedCount.ShouldBe(variants.Length);
        result.CorruptKeyCount.ShouldBe(0);
        result.FailedBlobCount.ShouldBe(0);
        result.MissingBlobCount.ShouldBe(0);

        // Provider delete count increased by exactly 6
        (_blobProvider.DeleteCount - initialDeleteCount).ShouldBe(variants.Length);

        // Database and storage assertions
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            foreach (var sc in seededScreenshots)
            {
                (await _screenshotRepository.FindAsync(sc.Id)).ShouldBeNull();
                _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeFalse();
            }
        }
    }
}
