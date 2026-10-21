using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.BlobStoring;
using Volo.Abp.Data;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-DAT-01: Workspace-scoped activity and time summary age-retention test suite.
/// Uses isolated SQLite in-memory database and synthetic ScriptedBlobProvider.
/// Exercises:
/// 1. Expired eligible activity without captures is deleted while same-age Workspace B survives.
/// 2. Fresh and exact-cutoff activities survive.
/// 3. Referenced activity with screenshots is preserved along with metadata and blob bytes.
/// 4. Foreign or corrupt capture reference prevents cascade and protects activity and blob bytes.
/// 5. Invalid or empty policy is rejected before any database operations.
/// 6. Bounded batching and keyset progress past leading preserved dependencies without infinite loops.
/// 7. Repeat cleanup is idempotent with zero candidates remaining.
/// 8. Archived workspace policy defaults to skip and allows explicit execution.
/// 9. Concrete project, membership, user, and subscription sentinels remain untouched.
/// 10. Cancellation token halts execution without database mutation.
/// 11. Non-existent workspace throws EntityNotFoundException.
/// </summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspaceActivityRetentionTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly Guid TestUserId = Guid.Parse("3e701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid UserB = Guid.Parse("3e701e62-0953-4dd3-910b-dc6cc93ccb02");

    private readonly ScriptedBlobProvider _blobProvider = new();
    private readonly IActivityRetentionService _retentionService;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceActivityRetentionTests()
    {
        _retentionService = GetRequiredService<IActivityRetentionService>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA, Guid ProjectB)> SeedFixtureAsync()
    {
        async Task EnsureUserAsync(Guid id, string userName, string email)
        {
            if (await _userManager.FindByIdAsync(id.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(id, userName, email));
            }
        }

        await EnsureUserAsync(TestUserId, "testuser", "testuser@dosi.test");
        await EnsureUserAsync(UserB, "userb", "userb@dosi.test");

        var wsA = await ServiceProvider.CreateWorkspaceAsync("ACT-RET-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("ACT-RET-WS-B");

        await ServiceProvider.AddWorkspaceMemberAsync(TestUserId, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);

        var projA = _guidGenerator.Create();
        var projB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _projectRepository.InsertAsync(new Project(projA, wsA, "Project-A"), autoSave: true);
                await _projectRepository.InsertAsync(new Project(projB, wsB, "Project-B"), autoSave: true);
            }
        });

        return (wsA, wsB, projA, projB);
    }

    private async Task<Activity> SeedActivityAsync(
        Guid workspaceId,
        Guid projectId,
        Guid userId,
        DateTime startedAt,
        int durationMinutes = 10)
    {
        var actId = _guidGenerator.Create();
        var clientActivityId = _guidGenerator.Create();
        var endedAt = startedAt.AddMinutes(durationMinutes);

        var activity = new Activity(
            id: actId,
            workspaceId: workspaceId,
            userId: userId,
            projectId: projectId,
            clientActivityId: clientActivityId,
            startedAt: startedAt,
            endedAt: endedAt)
        {
            Productivity = 85,
            MouseClicks = 150,
            KeyboardHits = 320,
            ActiveWindowsJson = "[\"Code.exe\"]",
            RunningProgramsJson = "[\"devenv.exe\"]",
            TimelineJson = "[{\"m\":0,\"k\":32}]"
        };

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _activityRepository.InsertAsync(activity, autoSave: true);
            }
        });

        return activity;
    }

    private async Task<Screenshot> SeedScreenshotAsync(
        Guid workspaceId,
        Guid activityId,
        DateTime capturedAt,
        string kind = Screenshot.ScreenKind)
    {
        var scId = _guidGenerator.Create();
        var storageUrl = $"activities/{activityId:D}/{kind}.jpg";
        var sc = new Screenshot(
            id: scId,
            workspaceId: workspaceId,
            activityId: activityId,
            storageUrl: storageUrl,
            capturedAt: capturedAt,
            sizeBytes: 2048,
            contentType: "image/jpeg",
            kind: kind);

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _screenshotRepository.InsertAsync(sc, autoSave: true);
            }
        });

        _blobProvider.Blobs[storageUrl] = new byte[] { 0xDE, 0xAD, 0xBE, 0xEF };
        return sc;
    }

    [Fact]
    public async Task Expired_Eligible_Activity_Without_Captures_Is_Deleted_While_SameAge_WorkspaceB_Survives()
    {
        var (wsA, wsB, projA, projB) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        // Seed expired activity in Workspace A (40 days old, no screenshots)
        var actA = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-40));

        // Seed expired activity in Workspace B (40 days old, same age, no screenshots)
        var actB = await SeedActivityAsync(wsB, projB, UserB, _clock.Now.AddDays(-40));

        var policyA = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            batchSize: 100,
            cutoff: cutoff);

        // Act
        var result = await _retentionService.CleanExpiredActivitiesAsync(policyA);

        // Assert
        result.WorkspaceId.ShouldBe(wsA);
        result.CandidateCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(1);
        result.PreservedReferencedCount.ShouldBe(0);
        result.HasMore.ShouldBeFalse();
        result.Skipped.ShouldBeFalse();

        // Workspace A expired row was deleted
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(actA.Id)).ShouldBeNull();
        }

        // Workspace B expired row survived untouched (cross-workspace isolation)
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var actBFromDb = await _activityRepository.FindAsync(actB.Id);
            actBFromDb.ShouldNotBeNull();
            actBFromDb.Productivity.ShouldBe(85);
            actBFromDb.MouseClicks.ShouldBe(150);
            actBFromDb.KeyboardHits.ShouldBe(320);
        }
    }

    [Fact]
    public async Task Fresh_And_ExactCutoff_Activities_Survive_Cleanup()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        // Seed activity at exact cutoff boundary (StartedAt == cutoff)
        var actExact = await SeedActivityAsync(wsA, projA, TestUserId, cutoff);

        // Seed fresh activity after cutoff (StartedAt > cutoff, 10 days old)
        var actFresh = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-10));

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            cutoff: cutoff);

        // Act
        var result = await _retentionService.CleanExpiredActivitiesAsync(policy);

        // Assert
        result.CandidateCount.ShouldBe(0);
        result.DeletedCount.ShouldBe(0);
        result.PreservedReferencedCount.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(actExact.Id)).ShouldNotBeNull();
            (await _activityRepository.FindAsync(actFresh.Id)).ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task Referenced_Activity_With_Screenshots_Is_Preserved_Along_With_Metadata_And_Blob_Bytes()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        // Seed expired activity in Workspace A (45 days old)
        var actReferenced = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-45));

        // Seed screenshot referencing this activity with fake blob bytes
        var sc = await SeedScreenshotAsync(wsA, actReferenced.Id, _clock.Now.AddDays(-45));

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            cutoff: cutoff);

        // Act
        var result = await _retentionService.CleanExpiredActivitiesAsync(policy);

        // Assert: activity was evaluated, but preserved due to capture metadata reference!
        result.CandidateCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(0);
        result.PreservedReferencedCount.ShouldBe(1);
        result.HasMore.ShouldBeFalse();

        // Database row survives intact
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var actFromDb = await _activityRepository.FindAsync(actReferenced.Id);
            actFromDb.ShouldNotBeNull();
            actFromDb.Productivity.ShouldBe(85);
            actFromDb.MouseClicks.ShouldBe(150);
            actFromDb.KeyboardHits.ShouldBe(320);

            // Screenshot metadata survives
            var scFromDb = await _screenshotRepository.FindAsync(sc.Id);
            scFromDb.ShouldNotBeNull();
        }

        // Blob storage bytes survive untouched (zero deletion calls)
        _blobProvider.Blobs.ContainsKey(sc.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Foreign_Or_Corrupt_Capture_Reference_Prevents_Cascade_And_Protects_Activity_And_Blob()
    {
        var (wsA, wsB, projA, _) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        // Seed expired activity in Workspace A (45 days old)
        var actForeignRef = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-45));

        // Seed screenshot pointing to actForeignRef, but with WorkspaceId = wsB (foreign/corrupt capture reference)
        var corruptSc = await SeedScreenshotAsync(wsB, actForeignRef.Id, _clock.Now.AddDays(-45));

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            cutoff: cutoff);

        // Act
        var result = await _retentionService.CleanExpiredActivitiesAsync(policy);

        // Assert: foreign capture reference is detected; activity row is preserved to prevent cascade loss!
        result.CandidateCount.ShouldBe(1);
        result.DeletedCount.ShouldBe(0);
        result.PreservedReferencedCount.ShouldBe(1);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(actForeignRef.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(corruptSc.Id)).ShouldNotBeNull();
        }

        _blobProvider.Blobs.ContainsKey(corruptSc.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Invalid_Or_Empty_Policy_Is_Rejected_Before_Any_Writes()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var act = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-45));

        // 1. Empty workspace ID rejected
        Should.Throw<ArgumentException>(() =>
            new ActivityRetentionPolicy(Guid.Empty, TimeSpan.FromDays(30)).Validate());

        // 2. Zero retention period rejected
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new ActivityRetentionPolicy(wsA, TimeSpan.Zero).Validate());

        // 3. Negative retention period rejected
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new ActivityRetentionPolicy(wsA, TimeSpan.FromDays(-10)).Validate());

        // 4. Batch size 0 rejected
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new ActivityRetentionPolicy(wsA, TimeSpan.FromDays(30), batchSize: 0).Validate());

        // 5. Batch size > 1000 rejected
        Should.Throw<ArgumentOutOfRangeException>(() =>
            new ActivityRetentionPolicy(wsA, TimeSpan.FromDays(30), batchSize: 1001).Validate());

        // Verify zero database modifications occurred
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(act.Id)).ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task Bounded_Batching_And_Keyset_Progress_Past_Leading_Preserved_Dependencies()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        // Seed 5 expired activities in chronological order:
        // Act 1: 50 days old, has screenshot (preserved)
        var act1 = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-50));
        var sc1 = await SeedScreenshotAsync(wsA, act1.Id, _clock.Now.AddDays(-50));

        // Act 2: 48 days old, has screenshot (preserved)
        var act2 = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-48));
        var sc2 = await SeedScreenshotAsync(wsA, act2.Id, _clock.Now.AddDays(-48));

        // Act 3: 46 days old, no screenshot (eligible)
        var act3 = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-46));

        // Act 4: 44 days old, no screenshot (eligible)
        var act4 = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-44));

        // Act 5: 42 days old, no screenshot (eligible)
        var act5 = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-42));

        // Page 1: batch size 2
        var policyPage1 = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            batchSize: 2,
            cutoff: cutoff);

        var result1 = await _retentionService.CleanExpiredActivitiesAsync(policyPage1);
        result1.CandidateCount.ShouldBe(2);
        result1.DeletedCount.ShouldBe(0);
        result1.PreservedReferencedCount.ShouldBe(2);
        result1.HasMore.ShouldBeTrue();
        result1.NextCursor.ShouldNotBeNull();
        result1.NextCursor.ActivityId.ShouldBe(act2.Id);

        // Page 2: continuing from NextCursor of Page 1
        var policyPage2 = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            batchSize: 2,
            cutoff: cutoff,
            cursor: result1.NextCursor);

        var result2 = await _retentionService.CleanExpiredActivitiesAsync(policyPage2);
        result2.CandidateCount.ShouldBe(2);
        result2.DeletedCount.ShouldBe(2); // act3 and act4 deleted!
        result2.PreservedReferencedCount.ShouldBe(0);
        result2.HasMore.ShouldBeTrue();
        result2.NextCursor.ShouldNotBeNull();
        result2.NextCursor.ActivityId.ShouldBe(act4.Id);

        // Page 3: continuing from NextCursor of Page 2
        var policyPage3 = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            batchSize: 2,
            cutoff: cutoff,
            cursor: result2.NextCursor);

        var result3 = await _retentionService.CleanExpiredActivitiesAsync(policyPage3);
        result3.CandidateCount.ShouldBe(1);
        result3.DeletedCount.ShouldBe(1); // act5 deleted!
        result3.PreservedReferencedCount.ShouldBe(0);
        result3.HasMore.ShouldBeFalse();
        result3.NextCursor.ShouldBeNull();

        // Verify final database state:
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            // Act 1 & 2 survive with screenshots and blobs intact
            (await _activityRepository.FindAsync(act1.Id)).ShouldNotBeNull();
            (await _activityRepository.FindAsync(act2.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(sc1.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(sc2.Id)).ShouldNotBeNull();

            // Act 3, 4, 5 are completely deleted
            (await _activityRepository.FindAsync(act3.Id)).ShouldBeNull();
            (await _activityRepository.FindAsync(act4.Id)).ShouldBeNull();
            (await _activityRepository.FindAsync(act5.Id)).ShouldBeNull();
        }

        _blobProvider.Blobs.ContainsKey(sc1.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(sc2.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Repeat_Cleanup_Is_Idempotent_With_Zero_Candidates()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var cutoff = _clock.Now.AddDays(-30);

        var act = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-40));

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30),
            cutoff: cutoff);

        // First run cleans the eligible row
        var firstResult = await _retentionService.CleanExpiredActivitiesAsync(policy);
        firstResult.DeletedCount.ShouldBe(1);

        // Second run finds 0 candidates
        var secondResult = await _retentionService.CleanExpiredActivitiesAsync(policy);
        secondResult.CandidateCount.ShouldBe(0);
        secondResult.DeletedCount.ShouldBe(0);
        secondResult.HasMore.ShouldBeFalse();
    }

    [Fact]
    public async Task Archived_Workspace_Policy_Defaults_To_Skip_And_Allows_Explicit_Execution()
    {
        var (_, _, projA, _) = await SeedFixtureAsync();
        var wsArchived = await ServiceProvider.CreateWorkspaceAsync("ARCHIVED-WS");
        await ServiceProvider.AddWorkspaceMemberAsync(TestUserId, WorkspaceRoles.Owner, wsArchived);

        // Archive the workspace
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsArchived);
            ws.Archive(TestUserId);
            await _workspaceRepository.UpdateAsync(ws, autoSave: true);
        });

        // Seed expired activity in the archived workspace
        var actArchived = await SeedActivityAsync(wsArchived, projA, TestUserId, _clock.Now.AddDays(-40));

        // 1. Default policy (AllowArchivedWorkspace = false): skips archived workspace
        var skipPolicy = new ActivityRetentionPolicy(
            workspaceId: wsArchived,
            retentionPeriod: TimeSpan.FromDays(30),
            allowArchivedWorkspace: false);

        var skipResult = await _retentionService.CleanExpiredActivitiesAsync(skipPolicy);
        skipResult.Skipped.ShouldBeTrue();
        skipResult.SkipReason.ShouldBe("Workspace is archived");
        skipResult.DeletedCount.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(actArchived.Id)).ShouldNotBeNull();
        }

        // 2. Explicit policy with AllowArchivedWorkspace = true: cleans expired activity
        var allowPolicy = new ActivityRetentionPolicy(
            workspaceId: wsArchived,
            retentionPeriod: TimeSpan.FromDays(30),
            allowArchivedWorkspace: true);

        var allowResult = await _retentionService.CleanExpiredActivitiesAsync(allowPolicy);
        allowResult.Skipped.ShouldBeFalse();
        allowResult.DeletedCount.ShouldBe(1);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(actArchived.Id)).ShouldBeNull();
        }
    }

    [Fact]
    public async Task Sentinels_For_Project_Membership_Subscription_And_User_Remain_Untouched()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var act = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-40));

        // Seed Plan and Subscription
        var planId = _guidGenerator.Create();
        var subId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                var plan = new Plan(planId, "Enterprise", pricePerUser: 50m, maxSeats: 5, trialDays: 14);
                await _planRepository.InsertAsync(plan, autoSave: true);

                var sub = new Subscription(subId, wsA, planId);
                await _subscriptionRepository.InsertAsync(sub, autoSave: true);
            }
        });

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30));

        var result = await _retentionService.CleanExpiredActivitiesAsync(policy);
        result.DeletedCount.ShouldBe(1);

        // Sentinels assertion: User, Project, Member, Plan, Subscription remain 100% intact!
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _userManager.FindByIdAsync(TestUserId.ToString())).ShouldNotBeNull();
            (await _projectRepository.FindAsync(projA)).ShouldNotBeNull();
            (await _memberRepository.FindAsync(m => m.WorkspaceId == wsA && m.UserId == TestUserId)).ShouldNotBeNull();
            (await _planRepository.FindAsync(planId)).ShouldNotBeNull();
            (await _subscriptionRepository.FindAsync(subId)).ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task Cancellation_Token_Halts_Execution_Without_Mutation()
    {
        var (wsA, _, projA, _) = await SeedFixtureAsync();
        var act = await SeedActivityAsync(wsA, projA, TestUserId, _clock.Now.AddDays(-40));

        var policy = new ActivityRetentionPolicy(
            workspaceId: wsA,
            retentionPeriod: TimeSpan.FromDays(30));

        using var cts = new CancellationTokenSource();
        cts.Cancel(); // Pre-canceled token

        await Should.ThrowAsync<OperationCanceledException>(async () =>
            await _retentionService.CleanExpiredActivitiesAsync(policy, cts.Token));

        // Verify entity was not deleted
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _activityRepository.FindAsync(act.Id)).ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task Non_Existent_Workspace_Throws_EntityNotFoundException()
    {
        var nonExistentWorkspace = Guid.NewGuid();
        var policy = new ActivityRetentionPolicy(
            workspaceId: nonExistentWorkspace,
            retentionPeriod: TimeSpan.FromDays(30));

        await Should.ThrowAsync<EntityNotFoundException>(async () =>
            await _retentionService.CleanExpiredActivitiesAsync(policy));
    }
}
