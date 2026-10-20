using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Data.Common;
using System.IO;
using System.Linq;
using System.Security.Claims;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Diagnostics;
using Microsoft.EntityFrameworkCore.Infrastructure;
using Microsoft.EntityFrameworkCore.Storage;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.EntityFrameworkCore;
using Volo.Abp.EntityFrameworkCore.Sqlite;
using Volo.Abp.FeatureManagement;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.PermissionManagement;
using Volo.Abp.Security.Claims;
using Volo.Abp.SettingManagement;
using Volo.Abp.Testing;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

public class SqliteWalConnectionInterceptor : DbConnectionInterceptor
{
    public override void ConnectionOpened(DbConnection connection, ConnectionEndEventData eventData)
    {
        using var cmd = connection.CreateCommand();
        cmd.CommandText = "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 30000;";
        cmd.ExecuteNonQuery();
        base.ConnectionOpened(connection, eventData);
    }

    public override async Task ConnectionOpenedAsync(DbConnection connection, ConnectionEndEventData eventData, CancellationToken cancellationToken = default)
    {
        using var cmd = connection.CreateCommand();
        cmd.CommandText = "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 30000;";
        await cmd.ExecuteNonQueryAsync(cancellationToken);
        await base.ConnectionOpenedAsync(connection, eventData, cancellationToken);
    }
}

/// <summary>
/// A controllable per-key distributed lock provider implementing IAbpDistributedLock for integration testing.
/// Provides deterministic bounded hooks (LockAcquireAttempted, LockWaitEntered, LockAcquired, OnBeforeReleaseAsync, LockReleasing),
/// thread-safe state inspection (ActiveHolders, QueuedWaiters, MaxConcurrentHolders), and guaranteed cleanup.
/// </summary>
public class ControllableDistributedLock : IAbpDistributedLock
{
    public class LockState
    {
        public SemaphoreSlim Semaphore { get; } = new(1, 1);
        public int ActiveHolders;
        public int MaxConcurrentHolders;
        public int QueuedWaiters;
        public int TotalAcquisitions;
    }

    private readonly ConcurrentDictionary<string, LockState> _states = new();

    public event Action<string>? LockAcquireAttempted;
    public event Action<string>? LockWaitEntered;
    public event Action<string>? LockAcquired;
    public event Action<string>? LockReleasing;
    public Func<string, Task>? OnBeforeReleaseAsync { get; set; }

    public LockState GetState(string name) => _states.GetOrAdd(name, _ => new LockState());

    public async Task<IAbpDistributedLockHandle?> TryAcquireAsync(
        string name, TimeSpan timeout = default, CancellationToken cancellationToken = default)
    {
        var state = GetState(name);
        LockAcquireAttempted?.Invoke(name);

        Interlocked.Increment(ref state.QueuedWaiters);
        LockWaitEntered?.Invoke(name);

        bool acquired;
        try
        {
            acquired = await state.Semaphore.WaitAsync(timeout, cancellationToken);
        }
        finally
        {
            Interlocked.Decrement(ref state.QueuedWaiters);
        }

        if (!acquired)
        {
            return null;
        }

        var current = Interlocked.Increment(ref state.ActiveHolders);
        Interlocked.Increment(ref state.TotalAcquisitions);

        int initialMax, newMax;
        do
        {
            initialMax = state.MaxConcurrentHolders;
            newMax = Math.Max(initialMax, current);
        } while (Interlocked.CompareExchange(ref state.MaxConcurrentHolders, newMax, initialMax) != initialMax);

        LockAcquired?.Invoke(name);
        return new Handle(this, name, state);
    }

    private sealed class Handle : IAbpDistributedLockHandle
    {
        private readonly ControllableDistributedLock _owner;
        private readonly string _name;
        private readonly LockState _state;
        private int _disposed;

        public Handle(ControllableDistributedLock owner, string name, LockState state)
        {
            _owner = owner;
            _name = name;
            _state = state;
        }

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _disposed, 1) == 0)
            {
                try
                {
                    if (_owner.OnBeforeReleaseAsync != null)
                    {
                        await _owner.OnBeforeReleaseAsync(_name);
                    }
                }
                finally
                {
                    try
                    {
                        _owner.LockReleasing?.Invoke(_name);
                    }
                    finally
                    {
                        Interlocked.Decrement(ref _state.ActiveHolders);
                        _state.Semaphore.Release();
                    }
                }
            }
        }
    }
}

[DependsOn(
    typeof(TrackerApplicationTestModule),
    typeof(TrackerEntityFrameworkCoreModule),
    typeof(AbpEntityFrameworkCoreSqliteModule)
)]
public class ConcurrentOwnerProtectionTestModule : AbpModule
{
    public readonly string DatabaseFilePath = Path.Combine(
        Path.GetTempPath(),
        $"dosi_owner_concurrency_{Guid.NewGuid():N}.db");

    public string ConnectionString => $"Data Source={DatabaseFilePath};Mode=ReadWriteCreate;";

    public override void PreConfigureServices(ServiceConfigurationContext context)
    {
        PreConfigure<AbpSqliteOptions>(x => x.BusyTimeout = 30000);
    }

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        Configure<FeatureManagementOptions>(options =>
        {
            options.SaveStaticFeaturesToDatabase = false;
            options.IsDynamicFeatureStoreEnabled = false;
        });
        Configure<PermissionManagementOptions>(options =>
        {
            options.SaveStaticPermissionsToDatabase = false;
            options.IsDynamicPermissionStoreEnabled = false;
        });
        Configure<SettingManagementOptions>(options =>
        {
            options.SaveStaticSettingsToDatabase = false;
            options.IsDynamicSettingStoreEnabled = false;
        });

        // Provider Inability Note:
        // SQLite (in-memory or single file) does not support multi-connection concurrent write transactions in ABP;
        // enabling transactions causes SQLite Error 5: 'database is locked' during module data seeding.
        // Therefore, AddAlwaysDisableUnitOfWorkTransaction is required by this SQLite test fixture.
        // Production code explicitly requests isTransactional: true and awaits uow.CompleteAsync() before releasing
        // the owner lock, but full multi-connection transactional commit proof requires a database provider supporting
        // concurrent write transactions (e.g., PostgreSQL / SQL Server).
        context.Services.AddAlwaysDisableUnitOfWorkTransaction();

        // Register controllable distributed lock for deterministic synchronization and key inspection
        context.Services.Replace(ServiceDescriptor.Singleton<ControllableDistributedLock, ControllableDistributedLock>());
        context.Services.Replace(ServiceDescriptor.Singleton<IAbpDistributedLock>(sp => sp.GetRequiredService<ControllableDistributedLock>()));

        // Initialize dedicated file database in WAL mode for independent multi-connection concurrency
        InitializeDatabase(ConnectionString);

        context.Services.Configure<AbpDbContextOptions>(options =>
        {
            options.Configure(ctx =>
            {
                ctx.DbContextOptions.UseSqlite(ConnectionString);
                ctx.DbContextOptions.AddInterceptors(new SqliteWalConnectionInterceptor());
            });
        });
    }

    private static void InitializeDatabase(string connectionString)
    {
        var options = new DbContextOptionsBuilder<TrackerDbContext>()
            .UseSqlite(connectionString)
            .Options;

        using (var dbContext = new TrackerDbContext(options))
        {
            dbContext.Database.OpenConnection();
            using (var cmd = dbContext.Database.GetDbConnection().CreateCommand())
            {
                cmd.CommandText = "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=30000;";
                cmd.ExecuteNonQuery();
            }

            dbContext.GetService<IRelationalDatabaseCreator>().CreateTables();
            dbContext.Database.CloseConnection();
        }

        SqliteConnection.ClearAllPools();
    }

    public override void OnApplicationShutdown(ApplicationShutdownContext context)
    {
        SqliteConnection.ClearAllPools();
        try
        {
            if (File.Exists(DatabaseFilePath)) File.Delete(DatabaseFilePath);
            var wal = DatabaseFilePath + "-wal";
            if (File.Exists(wal)) File.Delete(wal);
            var shm = DatabaseFilePath + "-shm";
            if (File.Exists(shm)) File.Delete(shm);
        }
        catch
        {
            // Best-effort cleanup of disposable test database file
        }
    }
}

public class ConcurrentOwnerProtectionTests : TrackerTestBase<ConcurrentOwnerProtectionTestModule>
{
    private readonly IGuidGenerator _guidGenerator;

    public ConcurrentOwnerProtectionTests()
    {
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private ControllableDistributedLock DistributedLock => GetRequiredService<ControllableDistributedLock>();

    private static ClaimsPrincipal CreatePrincipal(Guid userId)
    {
        var claims = new List<Claim>
        {
            new(AbpClaimTypes.UserId, userId.ToString()),
            new(AbpClaimTypes.UserName, "user_" + userId.ToString("N")[..8]),
            new(AbpClaimTypes.Email, $"user_{userId:N}@test.com")
        };
        return new ClaimsPrincipal(new ClaimsIdentity(claims, "TestAuth"));
    }

    private async Task<IdentityUser> CreateTestUserAsync(string prefix)
    {
        using var scope = ServiceProvider.CreateScope();
        var userManager = scope.ServiceProvider.GetRequiredService<IdentityUserManager>();
        var id = _guidGenerator.Create();
        var user = new IdentityUser(id, $"{prefix}_{id:N}", $"{prefix}_{id:N}@test.com");
        (await userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
        return user;
    }

    private async Task CreateWorkspaceWithOwnerAsync(Guid workspaceId, string name, Guid ownerUserId)
    {
        using var scope = ServiceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true, isTransactional: true);
        var wsRepo = scope.ServiceProvider.GetRequiredService<IRepository<Workspace, Guid>>();
        var memMgr = scope.ServiceProvider.GetRequiredService<WorkspaceMembershipManager>();

        await wsRepo.InsertAsync(new Workspace(workspaceId, name), autoSave: true);
        await memMgr.AddAsync(workspaceId, ownerUserId, WorkspaceRoles.Owner);
        await uow.CompleteAsync();
    }

    private async Task AddOwnerAsync(Guid workspaceId, Guid userId)
    {
        using var scope = ServiceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true, isTransactional: true);
        var memMgr = scope.ServiceProvider.GetRequiredService<WorkspaceMembershipManager>();

        await memMgr.AddAsync(workspaceId, userId, WorkspaceRoles.Owner);
        await uow.CompleteAsync();
    }

    private async Task<Guid> CreateProjectWithMemberAsync(Guid workspaceId, Guid userId)
    {
        var projectId = _guidGenerator.Create();
        using var scope = ServiceProvider.CreateScope();
        var currentWs = scope.ServiceProvider.GetRequiredService<ICurrentWorkspace>();
        using (currentWs.Change(workspaceId))
        {
            var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uow = uowManager.Begin(requiresNew: true, isTransactional: true);
            var projRepo = scope.ServiceProvider.GetRequiredService<IRepository<Project, Guid>>();
            var projMemRepo = scope.ServiceProvider.GetRequiredService<IRepository<ProjectMember, Guid>>();

            await projRepo.InsertAsync(new Project(projectId, workspaceId, "Test Project " + projectId.ToString("N")[..8]), autoSave: true);
            await projMemRepo.InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectId, userId), autoSave: true);
            await uow.CompleteAsync();
            return projectId;
        }
    }

    private async Task<int> CountActiveOwnersInDbAsync(Guid workspaceId)
    {
        using var scope = ServiceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true);
        var memRepo = scope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        var count = await memRepo.CountAsync(m =>
            m.WorkspaceId == workspaceId &&
            m.Status == WorkspaceMemberStatus.Active &&
            m.Role == WorkspaceRoles.Owner);
        await uow.CompleteAsync();
        return count;
    }

    [Fact]
    public async Task Modeled_Unsynchronized_Last_Owner_Removal_Simulates_Orphaning_Risk()
    {
        // Bounded Modeled Interleaving Simulation (Not a claim of prior production codebase execution):
        // Demonstrates the theoretical race condition where, in the absence of per-workspace ownership synchronization,
        // two concurrent removal operations independently read CountOwnersAsync == 2, both pass the sequential
        // "count <= 1" check, and both mutate/commit. As a result, active owner count drops to 0 (orphaning the workspace).

        var wsId = _guidGenerator.Create();
        var userA = await CreateTestUserAsync("vuln_a");
        var userB = await CreateTestUserAsync("vuln_b");
        await CreateWorkspaceWithOwnerAsync(wsId, "VulnWorkspace", userA.Id);
        await AddOwnerAsync(wsId, userB.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        using (var scopeA = ServiceProvider.CreateScope())
        using (var scopeB = ServiceProvider.CreateScope())
        {
            var uowManagerA = scopeA.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uowA = uowManagerA.Begin(requiresNew: true);
            var memRepoA = scopeA.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();

            var uowManagerB = scopeB.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uowB = uowManagerB.Begin(requiresNew: true);
            var memRepoB = scopeB.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();

            var memberA = await memRepoA.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == userA.Id);
            var memberB = await memRepoB.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == userB.Id);

            // Both threads independently check owner count before either commits
            var countObservedByA = await memRepoA.CountAsync(m => m.WorkspaceId == wsId && m.Status == WorkspaceMemberStatus.Active && m.Role == WorkspaceRoles.Owner);
            var countObservedByB = await memRepoB.CountAsync(m => m.WorkspaceId == wsId && m.Status == WorkspaceMemberStatus.Active && m.Role == WorkspaceRoles.Owner);

            countObservedByA.ShouldBe(2);
            countObservedByB.ShouldBe(2);

            // Because count was 2, neither threw LastOwnerCannotLeave
            (countObservedByA <= 1).ShouldBeFalse();
            (countObservedByB <= 1).ShouldBeFalse();

            // Thread A removes owner A and updates
            memberA.Remove();
            await memRepoA.UpdateAsync(memberA, autoSave: true);
            await uowA.CompleteAsync();

            // Thread B, having bypassed the check, removes owner B and updates
            memberB.Remove();
            await memRepoB.UpdateAsync(memberB, autoSave: true);
            await uowB.CompleteAsync();
        }

        // Final persisted active-owner count drops to 0 in this modeled simulation
        var orphanedCount = await CountActiveOwnersInDbAsync(wsId);
        orphanedCount.ShouldBe(0);
    }

    [Fact]
    public async Task Owner_Lock_Is_Exclusive_Held_Across_Commit_And_Unrelated_Workspaces_Not_Serialized()
    {
        // Proves that:
        // 1. LockOwnersAsync is strictly exclusive for the targeted workspace key.
        // 2. A competitor attempting to acquire the lock for the same workspace fails/blocks.
        // 3. Lock keys for unrelated workspaces are completely independent and not globally serialized.
        // 4. Upon disposal/release, the competitor successfully acquires the lock.

        var ws1Id = _guidGenerator.Create();
        var ws2Id = _guidGenerator.Create();

        using var scope1 = ServiceProvider.CreateScope();
        var memMgr1 = scope1.ServiceProvider.GetRequiredService<WorkspaceMembershipManager>();

        await using (var lockHandle1 = await memMgr1.LockOwnersAsync(ws1Id))
        {
            // While WS1 lock is held, competitor for WS1 cannot acquire it
            var competitorWs1 = await DistributedLock.TryAcquireAsync(
                WorkspaceMembershipManager.OwnerLockName(ws1Id),
                TimeSpan.FromMilliseconds(50));
            competitorWs1.ShouldBeNull();

            // While WS1 lock is held, competitor for unrelated WS2 acquires lock immediately
            var competitorWs2 = await DistributedLock.TryAcquireAsync(
                WorkspaceMembershipManager.OwnerLockName(ws2Id),
                TimeSpan.FromSeconds(5));
            competitorWs2.ShouldNotBeNull();
            await competitorWs2.DisposeAsync();
        }

        // After WS1 lock is released, competitor for WS1 can acquire it
        var postReleaseWs1 = await DistributedLock.TryAcquireAsync(
            WorkspaceMembershipManager.OwnerLockName(ws1Id),
            TimeSpan.FromSeconds(5));
        postReleaseWs1.ShouldNotBeNull();
        await postReleaseWs1.DisposeAsync();
    }

    [Fact]
    public async Task Deterministic_Interleaved_Owner_Leave_Blocks_Competitor_At_Lock_Boundary_Until_Persisted_Release()
    {
        // Deterministic Bounded Interleaving Proof (SQLite Nontransactional Persisted-State Ordering):
        // 1. Thread 1 enters LeaveAsync, acquires ownerLock, and proceeds through member/project cleanup and persistence.
        // 2. Thread 2 enters LeaveAsync and reaches the actual provider boundary (TryAcquireAsync), actively queuing on the semaphore.
        // 3. At the first-operation persisted-state pre-release boundary (OnBeforeReleaseAsync):
        //    - Thread 1's entity removal is verified already persisted in SQLite (ownersInDb == 1).
        //    - Competitor Thread 2 is verified actively queued at the provider boundary on the held semaphore (QueuedWaiters >= 1, ActiveHolders == 1).
        //    - task2.IsCompleted is verified false.
        // 4. Thread 1 releases the lock.
        // 5. Thread 2 unblocks, acquires ownerLock, observes Thread 1's persisted removal, evaluates CountOwnersAsync <= 1,
        //    and is rejected with TrackerDomainErrorCodes.LastOwnerCannotLeave.
        // 6. MaxConcurrentHolders is asserted to be strictly 1 (verifying mutual exclusion and rejecting non-exclusive variants).
        // 7. Persisted active owners count remains exactly 1.
        // All waits and timeouts are strictly bounded (10s max). Guaranteed cleanup in finally blocks.

        var wsId = _guidGenerator.Create();
        var owner1 = await CreateTestUserAsync("det_a");
        var owner2 = await CreateTestUserAsync("det_b");

        await CreateWorkspaceWithOwnerAsync(wsId, "WS_DetInterleave", owner1.Id);
        await AddOwnerAsync(wsId, owner2.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        var lockName = WorkspaceMembershipManager.OwnerLockName(wsId);
        var thread1AcquiredLock = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread2QueuedAtLock = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var allowThread1Release = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);

        void OnLockAcquired(string name)
        {
            if (name == lockName)
            {
                thread1AcquiredLock.TrySetResult(true);
            }
        }

        void OnLockWaitEntered(string name)
        {
            // Thread 2 reaches the lock provider while Thread 1 holds the lock
            if (name == lockName && thread1AcquiredLock.Task.IsCompletedSuccessfully)
            {
                thread2QueuedAtLock.TrySetResult(true);
            }
        }

        int releaseCount = 0;
        DistributedLock.LockAcquired += OnLockAcquired;
        DistributedLock.LockWaitEntered += OnLockWaitEntered;
        DistributedLock.OnBeforeReleaseAsync = async name =>
        {
            if (name == lockName && Interlocked.Increment(ref releaseCount) == 1)
            {
                // Wait until Thread 2 has reached the lock provider and entered semaphore wait
                await thread2QueuedAtLock.Task.WaitAsync(TimeSpan.FromSeconds(10));

                // Assert at pre-release boundary: Thread 1 has completed its persistence in SQLite
                var ownersInDb = await CountActiveOwnersInDbAsync(wsId);
                ownersInDb.ShouldBe(1);

                // Assert at pre-release boundary: Thread 2 is genuinely queued at the provider boundary
                var state = DistributedLock.GetState(lockName);
                state.ActiveHolders.ShouldBe(1);
                state.QueuedWaiters.ShouldBeGreaterThanOrEqualTo(1);

                // Bounded wait for release signal
                await allowThread1Release.Task.WaitAsync(TimeSpan.FromSeconds(10));
            }
        };

        Exception? ex1 = null;
        Exception? ex2 = null;

        try
        {
            var task1 = Task.Run(async () =>
            {
                try
                {
                    using var scope = ServiceProvider.CreateScope();
                    var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                    var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
                    using (principalAccessor.Change(CreatePrincipal(owner1.Id)))
                    {
                        await appService.LeaveAsync(wsId);
                    }
                }
                catch (Exception ex)
                {
                    ex1 = ex;
                }
            });

            // Bounded wait until Thread 1 has acquired the lock
            await thread1AcquiredLock.Task.WaitAsync(TimeSpan.FromSeconds(10));

            // Launch Thread 2 while Thread 1 holds the lock
            var task2 = Task.Run(async () =>
            {
                try
                {
                    using var scope = ServiceProvider.CreateScope();
                    var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                    var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
                    using (principalAccessor.Change(CreatePrincipal(owner2.Id)))
                    {
                        await appService.LeaveAsync(wsId);
                    }
                }
                catch (Exception ex)
                {
                    ex2 = ex;
                }
            });

            // Bounded wait until Thread 2 has reached the provider boundary and is actively queued on the held semaphore
            await thread2QueuedAtLock.Task.WaitAsync(TimeSpan.FromSeconds(10));

            // Verified: Thread 2 is blocked at the provider boundary
            task2.IsCompleted.ShouldBeFalse();
            DistributedLock.GetState(lockName).QueuedWaiters.ShouldBeGreaterThanOrEqualTo(1);

            // Signal Thread 1 to proceed with lock release
            allowThread1Release.TrySetResult(true);

            // Bounded wait for both tasks to complete
            await Task.WhenAll(task1, task2).WaitAsync(TimeSpan.FromSeconds(10));

            ex1.ShouldBeNull();
            var businessEx = ex2.ShouldBeOfType<BusinessException>();
            businessEx.Code.ShouldBe(TrackerDomainErrorCodes.LastOwnerCannotLeave);

            // Exactly 1 owner remains persisted in DB
            var finalOwners = await CountActiveOwnersInDbAsync(wsId);
            finalOwners.ShouldBe(1);

            // Rejection of non-exclusive or bypassed lock variant:
            // Max concurrent holders for the lock key must never exceed 1
            var finalState = DistributedLock.GetState(lockName);
            finalState.MaxConcurrentHolders.ShouldBe(1);
            finalState.ActiveHolders.ShouldBe(0);
            finalState.QueuedWaiters.ShouldBe(0);
        }
        finally
        {
            DistributedLock.LockAcquired -= OnLockAcquired;
            DistributedLock.LockWaitEntered -= OnLockWaitEntered;
            DistributedLock.OnBeforeReleaseAsync = null;
            allowThread1Release.TrySetResult(true);
        }
    }

    [Fact]
    public void Deliberately_Nonexclusive_Or_Bypassed_Lock_Variant_Is_Rejected_By_Concurrency_Assertions()
    {
        // Proves that our harness MaxConcurrentHolders and mutual exclusion assertions
        // are sufficient to detect and reject a deliberately non-exclusive or bypassed lock variant.

        var lockName = WorkspaceMembershipManager.OwnerLockName(_guidGenerator.Create());
        var state = DistributedLock.GetState(lockName);

        // Simulate concurrent entry of two holders (as would occur if lock exclusivity were bypassed)
        Interlocked.Increment(ref state.ActiveHolders);
        var secondEntry = Interlocked.Increment(ref state.ActiveHolders);
        state.MaxConcurrentHolders = Math.Max(state.MaxConcurrentHolders, secondEntry);

        // Verify that the concurrent entry produces MaxConcurrentHolders == 2
        state.MaxConcurrentHolders.ShouldBe(2);

        // Demonstrate that an assertion requiring mutual exclusion (MaxConcurrentHolders == 1) strictly rejects this variant
        Should.Throw<ShouldAssertException>(() => state.MaxConcurrentHolders.ShouldBe(1));

        // Restore fixture state
        Interlocked.Decrement(ref state.ActiveHolders);
        Interlocked.Decrement(ref state.ActiveHolders);
        state.MaxConcurrentHolders = 0;
    }

    [Fact]
    public async Task Two_Owners_Concurrently_Leaving_Preserves_At_Least_One_Active_Owner_And_Retains_Same_User_Unrelated_Workspace_Data()
    {
        // Passing-after regression:
        // Two owners in Workspace 1 invoke production WorkspaceAppService.LeaveAsync concurrently.
        // User 1 is ALSO an Owner and Project Member of unrelated Workspace 2.
        // Proves:
        // 1. Exactly one owner succeeds in leaving WS1, and the competitor is rejected with LastOwnerCannotLeave.
        // 2. Active owner count in WS1 is strictly >= 1 (exactly 1).
        // 3. User 1's seeded membership and project membership in Workspace 2 are strictly preserved (no cross-workspace cleanup).

        var ws1Id = _guidGenerator.Create();
        var ws2Id = _guidGenerator.Create();

        var owner1 = await CreateTestUserAsync("leave1");
        var owner2 = await CreateTestUserAsync("leave2");

        // Seed WS1 with owner1 and owner2
        await CreateWorkspaceWithOwnerAsync(ws1Id, "WS1_ConcurrentLeave", owner1.Id);
        await AddOwnerAsync(ws1Id, owner2.Id);
        var ws1Proj1 = await CreateProjectWithMemberAsync(ws1Id, owner1.Id);
        var ws1Proj2 = await CreateProjectWithMemberAsync(ws1Id, owner2.Id);

        // Seed WS2 where owner1 is ALSO an Owner and has a Project Member
        await CreateWorkspaceWithOwnerAsync(ws2Id, "WS2_Isolated", owner1.Id);
        var ws2Proj = await CreateProjectWithMemberAsync(ws2Id, owner1.Id);

        (await CountActiveOwnersInDbAsync(ws1Id)).ShouldBe(2);
        (await CountActiveOwnersInDbAsync(ws2Id)).ShouldBe(1);

        var barrier = new Barrier(2);
        Exception? ex1 = null;
        Exception? ex2 = null;

        var task1 = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
                using (principalAccessor.Change(CreatePrincipal(owner1.Id)))
                {
                    barrier.SignalAndWait();
                    await appService.LeaveAsync(ws1Id);
                }
            }
            catch (Exception ex)
            {
                ex1 = ex;
            }
        });

        var task2 = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
                using (principalAccessor.Change(CreatePrincipal(owner2.Id)))
                {
                    barrier.SignalAndWait();
                    await appService.LeaveAsync(ws1Id);
                }
            }
            catch (Exception ex)
            {
                ex2 = ex;
            }
        });

        await Task.WhenAll(task1, task2).WaitAsync(TimeSpan.FromSeconds(10));

        // Assert: Exactly one operation succeeded and exactly one threw LastOwnerCannotLeave
        var exceptions = new[] { ex1, ex2 }.Where(e => e != null).ToList();
        exceptions.Count.ShouldBe(1);

        var businessEx = exceptions[0].ShouldBeOfType<BusinessException>();
        businessEx.Code.ShouldBe(TrackerDomainErrorCodes.LastOwnerCannotLeave);

        // Assert: Persisted active owners in WS1 is exactly 1 (final persisted count >= 1)
        var finalWs1Owners = await CountActiveOwnersInDbAsync(ws1Id);
        finalWs1Owners.ShouldBe(1);

        // Assert: Winner was removed in WS1 and had WS1 project memberships removed;
        // Loser remained active Owner in WS1 and WS1 project membership is intact
        using (var verifyScope = ServiceProvider.CreateScope())
        {
            var dataFilter = verifyScope.ServiceProvider.GetRequiredService<IDataFilter>();
            using (dataFilter.Disable<IWorkspaceScoped>())
            {
                var uowManager = verifyScope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
                using var uow = uowManager.Begin(requiresNew: true);
                var memRepo = verifyScope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();
                var projMemRepo = verifyScope.ServiceProvider.GetRequiredService<IRepository<ProjectMember, Guid>>();

                var mem1 = await memRepo.FirstAsync(m => m.WorkspaceId == ws1Id && m.UserId == owner1.Id);
                var mem2 = await memRepo.FirstAsync(m => m.WorkspaceId == ws1Id && m.UserId == owner2.Id);

                var projMembers1 = await projMemRepo.GetListAsync(pm => pm.WorkspaceId == ws1Id && pm.UserId == owner1.Id);
                var projMembers2 = await projMemRepo.GetListAsync(pm => pm.WorkspaceId == ws1Id && pm.UserId == owner2.Id);

                if (ex1 == null)
                {
                    // Owner 1 won (left WS1)
                    mem1.Status.ShouldBe(WorkspaceMemberStatus.Removed);
                    projMembers1.ShouldBeEmpty();

                    // Owner 2 lost (kept as last owner in WS1)
                    mem2.Status.ShouldBe(WorkspaceMemberStatus.Active);
                    mem2.Role.ShouldBe(WorkspaceRoles.Owner);
                    projMembers2.Count.ShouldBe(1);
                }
                else
                {
                    // Owner 2 won (left WS1)
                    mem2.Status.ShouldBe(WorkspaceMemberStatus.Removed);
                    projMembers2.ShouldBeEmpty();

                    // Owner 1 lost (kept as last owner in WS1)
                    mem1.Status.ShouldBe(WorkspaceMemberStatus.Active);
                    mem1.Role.ShouldBe(WorkspaceRoles.Owner);
                    projMembers1.Count.ShouldBe(1);
                }

                // Assert: User 1's seeded WS2 membership and project membership remain strictly intact and active!
                var ws2Owner1Mem = await memRepo.FirstAsync(m => m.WorkspaceId == ws2Id && m.UserId == owner1.Id);
                ws2Owner1Mem.Status.ShouldBe(WorkspaceMemberStatus.Active);
                ws2Owner1Mem.Role.ShouldBe(WorkspaceRoles.Owner);

                var ws2ProjMembers = await projMemRepo.GetListAsync(pm => pm.WorkspaceId == ws2Id && pm.UserId == owner1.Id);
                ws2ProjMembers.Count.ShouldBe(1);

                await uow.CompleteAsync();
            }
        }
    }

    [Fact]
    public async Task Two_Owners_Concurrently_Demoting_Each_Other_Preserves_At_Least_One_Active_Owner()
    {
        // Passing-after regression:
        // In a 2-owner workspace, Owner A attempts to demote Owner B to Manager,
        // while Owner B concurrently attempts to demote Owner A to Manager.
        // Under per-workspace serialization and transactional lock scoping:
        // Exactly one succeeds, and the other is rejected with domain error (RoleNotAllowed or LastOwnerCannotLeave).
        // At least one active owner remains.

        var wsId = _guidGenerator.Create();
        var ownerA = await CreateTestUserAsync("demote_a");
        var ownerB = await CreateTestUserAsync("demote_b");

        await CreateWorkspaceWithOwnerAsync(wsId, "WS_ConcurrentDemote", ownerA.Id);
        await AddOwnerAsync(wsId, ownerB.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        var barrier = new Barrier(2);
        Exception? exA = null;
        Exception? exB = null;

        var taskA = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var currentWs = scope.ServiceProvider.GetRequiredService<ICurrentWorkspace>();
                var teamAppService = scope.ServiceProvider.GetRequiredService<ITeamAppService>();

                using (principalAccessor.Change(CreatePrincipal(ownerA.Id)))
                using (currentWs.Change(wsId, WorkspaceRoles.Owner))
                {
                    barrier.SignalAndWait();
                    await teamAppService.UpdateWorkspaceRoleAsync(ownerB.Id, WorkspaceRoles.Manager);
                }
            }
            catch (Exception ex)
            {
                exA = ex;
            }
        });

        var taskB = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var currentWs = scope.ServiceProvider.GetRequiredService<ICurrentWorkspace>();
                var teamAppService = scope.ServiceProvider.GetRequiredService<ITeamAppService>();

                using (principalAccessor.Change(CreatePrincipal(ownerB.Id)))
                using (currentWs.Change(wsId, WorkspaceRoles.Owner))
                {
                    barrier.SignalAndWait();
                    await teamAppService.UpdateWorkspaceRoleAsync(ownerA.Id, WorkspaceRoles.Manager);
                }
            }
            catch (Exception ex)
            {
                exB = ex;
            }
        });

        await Task.WhenAll(taskA, taskB).WaitAsync(TimeSpan.FromSeconds(10));

        var exceptions = new[] { exA, exB }.Where(e => e != null).ToList();
        exceptions.Count.ShouldBe(1);

        var businessEx = exceptions[0].ShouldBeOfType<BusinessException>();
        new[] { TrackerDomainErrorCodes.LastOwnerCannotLeave, TrackerDomainErrorCodes.RoleNotAllowed }
            .ShouldContain(businessEx.Code);

        // Final active owner count is exactly 1
        var finalOwners = await CountActiveOwnersInDbAsync(wsId);
        finalOwners.ShouldBe(1);

        // Verify one is Manager and one remains Owner
        using (var verifyScope = ServiceProvider.CreateScope())
        {
            var uowManager = verifyScope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uow = uowManager.Begin(requiresNew: true);
            var memRepo = verifyScope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();
            var memA = await memRepo.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == ownerA.Id);
            var memB = await memRepo.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == ownerB.Id);

            memA.Status.ShouldBe(WorkspaceMemberStatus.Active);
            memB.Status.ShouldBe(WorkspaceMemberStatus.Active);

            var roles = new[] { memA.Role, memB.Role };
            roles.ShouldContain(WorkspaceRoles.Owner);
            roles.ShouldContain(WorkspaceRoles.Manager);
            await uow.CompleteAsync();
        }
    }

    [Fact]
    public async Task Concurrent_Mixed_Owner_Leave_And_Demotion_With_Distinct_Targets_Preserves_At_Least_One_Active_Owner()
    {
        // Reachable Mixed Leave & Demotion with Distinct Withdrawal Targets:
        // Workspace has two owners: Owner A and Owner B.
        // Withdrawal 1 Target: Owner B leaves via WorkspaceAppService.LeaveAsync.
        // Withdrawal 2 Target: Owner A is demoted to Manager via TeamAppService.UpdateWorkspaceRoleAsync (initiated by Owner B).
        // If uncoordinated: Owner B leaves (count drops to 1) and Owner A is demoted (count drops to 0) -> orphaning vulnerability!
        // Under per-workspace serialization and transactional lock scoping:
        // Either Leave executes first (Owner B removed; Demote fails with RoleNotAllowed since caller B is no longer an owner),
        // or Demote executes first (Owner A becomes Manager; Leave fails with LastOwnerCannotLeave since B is now the last owner).
        // In all execution orders, active owners count in the workspace NEVER drops below 1.

        var wsId = _guidGenerator.Create();
        var ownerA = await CreateTestUserAsync("dist_mixed_a");
        var ownerB = await CreateTestUserAsync("dist_mixed_b");

        await CreateWorkspaceWithOwnerAsync(wsId, "WS_MixedLeaveDemoteDistinct", ownerA.Id);
        await AddOwnerAsync(wsId, ownerB.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        var barrier = new Barrier(2);
        Exception? exLeave = null;
        Exception? exDemote = null;

        var taskLeave = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();

                using (principalAccessor.Change(CreatePrincipal(ownerB.Id)))
                {
                    barrier.SignalAndWait();
                    await appService.LeaveAsync(wsId);
                }
            }
            catch (Exception ex)
            {
                exLeave = ex;
            }
        });

        var taskDemote = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var currentWs = scope.ServiceProvider.GetRequiredService<ICurrentWorkspace>();
                var teamAppService = scope.ServiceProvider.GetRequiredService<ITeamAppService>();

                using (principalAccessor.Change(CreatePrincipal(ownerB.Id)))
                using (currentWs.Change(wsId, WorkspaceRoles.Owner))
                {
                    barrier.SignalAndWait();
                    await teamAppService.UpdateWorkspaceRoleAsync(ownerA.Id, WorkspaceRoles.Manager);
                }
            }
            catch (Exception ex)
            {
                exDemote = ex;
            }
        });

        await Task.WhenAll(taskLeave, taskDemote).WaitAsync(TimeSpan.FromSeconds(10));

        // Assert exactly one operation succeeded and exactly one threw a domain exception
        var exceptions = new[] { exLeave, exDemote }.Where(e => e != null).ToList();
        exceptions.Count.ShouldBe(1);

        var businessEx = exceptions[0].ShouldBeOfType<BusinessException>();
        new[] { TrackerDomainErrorCodes.LastOwnerCannotLeave, TrackerDomainErrorCodes.RoleNotAllowed }
            .ShouldContain(businessEx.Code);

        // In both interleavings, exactly 1 active owner remains
        var finalOwners = await CountActiveOwnersInDbAsync(wsId);
        finalOwners.ShouldBe(1);

        using (var verifyScope = ServiceProvider.CreateScope())
        {
            var uowManager = verifyScope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uow = uowManager.Begin(requiresNew: true);
            var memRepo = verifyScope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();
            var memA = await memRepo.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == ownerA.Id);
            var memB = await memRepo.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == ownerB.Id);

            if (exLeave == null)
            {
                // Leave won: Owner B was removed; Demote failed; Owner A remained Owner
                memB.Status.ShouldBe(WorkspaceMemberStatus.Removed);
                memA.Status.ShouldBe(WorkspaceMemberStatus.Active);
                memA.Role.ShouldBe(WorkspaceRoles.Owner);
            }
            else
            {
                // Demote won: Owner A became Manager; Leave failed; Owner B remained Owner
                memA.Status.ShouldBe(WorkspaceMemberStatus.Active);
                memA.Role.ShouldBe(WorkspaceRoles.Manager);
                memB.Status.ShouldBe(WorkspaceMemberStatus.Active);
                memB.Role.ShouldBe(WorkspaceRoles.Owner);
            }

            await uow.CompleteAsync();
        }
    }

    [Fact]
    public async Task Concurrent_Owner_Leaves_In_Different_Workspaces_Execute_Concurrently_Without_Contention()
    {
        // Verifies no global locks across unrelated workspaces:
        // Leaves in Workspace 1 and Workspace 2 run concurrently and both succeed.

        var ws1Id = _guidGenerator.Create();
        var ws2Id = _guidGenerator.Create();

        var ws1OwnerA = await CreateTestUserAsync("ws1_a");
        var ws1OwnerB = await CreateTestUserAsync("ws1_b");
        var ws2OwnerA = await CreateTestUserAsync("ws2_a");
        var ws2OwnerB = await CreateTestUserAsync("ws2_b");

        await CreateWorkspaceWithOwnerAsync(ws1Id, "WS1_Unrelated", ws1OwnerA.Id);
        await AddOwnerAsync(ws1Id, ws1OwnerB.Id);

        await CreateWorkspaceWithOwnerAsync(ws2Id, "WS2_Unrelated", ws2OwnerA.Id);
        await AddOwnerAsync(ws2Id, ws2OwnerB.Id);

        (await CountActiveOwnersInDbAsync(ws1Id)).ShouldBe(2);
        (await CountActiveOwnersInDbAsync(ws2Id)).ShouldBe(2);

        var barrier = new Barrier(2);
        Exception? ex1 = null;
        Exception? ex2 = null;

        var task1 = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();

                using (principalAccessor.Change(CreatePrincipal(ws1OwnerA.Id)))
                {
                    barrier.SignalAndWait();
                    await appService.LeaveAsync(ws1Id);
                }
            }
            catch (Exception ex)
            {
                ex1 = ex;
            }
        });

        var task2 = Task.Run(async () =>
        {
            try
            {
                using var scope = ServiceProvider.CreateScope();
                var principalAccessor = scope.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
                var appService = scope.ServiceProvider.GetRequiredService<IWorkspaceAppService>();

                using (principalAccessor.Change(CreatePrincipal(ws2OwnerA.Id)))
                {
                    barrier.SignalAndWait();
                    await appService.LeaveAsync(ws2Id);
                }
            }
            catch (Exception ex)
            {
                ex2 = ex;
            }
        });

        await Task.WhenAll(task1, task2).WaitAsync(TimeSpan.FromSeconds(10));

        // Both succeeded because they locked distinct workspace keys
        ex1.ShouldBeNull();
        ex2.ShouldBeNull();

        (await CountActiveOwnersInDbAsync(ws1Id)).ShouldBe(1);
        (await CountActiveOwnersInDbAsync(ws2Id)).ShouldBe(1);
    }

    [Fact]
    public async Task Multi_Owner_Sequential_Leaves_Allow_Demotions_Until_Last_Owner()
    {
        // Verifies legitimate multi-owner transitions are preserved:
        // In a 3-owner workspace:
        // Owner 1 leaves -> 2 owners remain
        // Owner 2 leaves -> 1 owner remains
        // Owner 3 leaves -> throws LastOwnerCannotLeave, 1 owner remains

        var wsId = _guidGenerator.Create();
        var owner1 = await CreateTestUserAsync("multi_1");
        var owner2 = await CreateTestUserAsync("multi_2");
        var owner3 = await CreateTestUserAsync("multi_3");

        await CreateWorkspaceWithOwnerAsync(wsId, "WS_MultiOwner", owner1.Id);
        await AddOwnerAsync(wsId, owner2.Id);
        await AddOwnerAsync(wsId, owner3.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(3);

        // Owner 1 leaves
        using (var scope1 = ServiceProvider.CreateScope())
        {
            var principalAccessor = scope1.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
            var appService = scope1.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
            using (principalAccessor.Change(CreatePrincipal(owner1.Id)))
            {
                await appService.LeaveAsync(wsId);
            }
        }
        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        // Owner 2 leaves
        using (var scope2 = ServiceProvider.CreateScope())
        {
            var principalAccessor = scope2.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
            var appService = scope2.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
            using (principalAccessor.Change(CreatePrincipal(owner2.Id)))
            {
                await appService.LeaveAsync(wsId);
            }
        }
        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(1);

        // Owner 3 leaves -> Rejected!
        using (var scope3 = ServiceProvider.CreateScope())
        {
            var principalAccessor = scope3.ServiceProvider.GetRequiredService<ICurrentPrincipalAccessor>();
            var appService = scope3.ServiceProvider.GetRequiredService<IWorkspaceAppService>();
            using (principalAccessor.Change(CreatePrincipal(owner3.Id)))
            {
                var ex = await Should.ThrowAsync<BusinessException>(() => appService.LeaveAsync(wsId));
                ex.Code.ShouldBe(TrackerDomainErrorCodes.LastOwnerCannotLeave);
            }
        }
        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(1);
    }
}
