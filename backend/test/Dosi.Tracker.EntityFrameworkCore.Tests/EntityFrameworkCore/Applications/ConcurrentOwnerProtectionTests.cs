using System;
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

        context.Services.AddAlwaysDisableUnitOfWorkTransaction();

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
    public async Task Unsynchronized_Concurrent_Last_Owner_Removal_Demonstrates_Orphaning_Vulnerability()
    {
        // Demonstrates the failing-before vulnerability:
        // Prior to per-workspace serialization and transactional lock scoping,
        // two concurrent removal operations independently read CountOwnersAsync == 2,
        // both pass the sequential "count <= 1" check, and both mutate/commit.
        // As a result, active owner count drops to 0 (orphaning the workspace).

        var wsId = _guidGenerator.Create();
        var userA = await CreateTestUserAsync("vuln_a");
        var userB = await CreateTestUserAsync("vuln_b");
        await CreateWorkspaceWithOwnerAsync(wsId, "VulnWorkspace", userA.Id);
        await AddOwnerAsync(wsId, userB.Id);

        (await CountActiveOwnersInDbAsync(wsId)).ShouldBe(2);

        // Simulate the exact unsynchronized interleaving:
        // Thread A loads membership and counts owners -> sees 2
        // Thread B loads membership and counts owners -> sees 2
        // Thread A passes check (2 <= 1 is false), removes owner A, saves
        // Thread B passes check (2 <= 1 was false), removes owner B, saves

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

            // Step 1: Both threads independently check owner count before either commits
            var countObservedByA = await memRepoA.CountAsync(m => m.WorkspaceId == wsId && m.Status == WorkspaceMemberStatus.Active && m.Role == WorkspaceRoles.Owner);
            var countObservedByB = await memRepoB.CountAsync(m => m.WorkspaceId == wsId && m.Status == WorkspaceMemberStatus.Active && m.Role == WorkspaceRoles.Owner);

            countObservedByA.ShouldBe(2);
            countObservedByB.ShouldBe(2);

            // Step 2: Because count was 2, neither threw LastOwnerCannotLeave
            (countObservedByA <= 1).ShouldBeFalse();
            (countObservedByB <= 1).ShouldBeFalse();

            // Step 3: Thread A removes owner A and updates
            memberA.Remove();
            await memRepoA.UpdateAsync(memberA, autoSave: true);
            await uowA.CompleteAsync();

            // Step 4: Thread B, having bypassed the check, removes owner B and updates
            memberB.Remove();
            await memRepoB.UpdateAsync(memberB, autoSave: true);
            await uowB.CompleteAsync();
        }

        // Final persisted active-owner count drops to 0 -> proves the unsynchronized race orphaning vulnerability!
        var orphanedCount = await CountActiveOwnersInDbAsync(wsId);
        orphanedCount.ShouldBe(0);
    }

    [Fact]
    public async Task Two_Owners_Concurrently_Leaving_Preserves_At_Least_One_Active_Owner_And_Rejects_Losing_Owner()
    {
        // Passing-after regression:
        // Two owners in Workspace 1 invoke production WorkspaceAppService.LeaveAsync concurrently.
        // A second unrelated Workspace 2 with its own owner and project is created to verify
        // that per-workspace locking preserves isolation and causes no unintended cross-workspace cleanup.

        var ws1Id = _guidGenerator.Create();
        var ws2Id = _guidGenerator.Create();

        var owner1 = await CreateTestUserAsync("leave1");
        var owner2 = await CreateTestUserAsync("leave2");
        var ws2Owner = await CreateTestUserAsync("ws2_owner");

        await CreateWorkspaceWithOwnerAsync(ws1Id, "WS1_ConcurrentLeave", owner1.Id);
        await AddOwnerAsync(ws1Id, owner2.Id);
        var ws1Proj1 = await CreateProjectWithMemberAsync(ws1Id, owner1.Id);
        var ws1Proj2 = await CreateProjectWithMemberAsync(ws1Id, owner2.Id);

        await CreateWorkspaceWithOwnerAsync(ws2Id, "WS2_Isolated", ws2Owner.Id);
        var ws2Proj = await CreateProjectWithMemberAsync(ws2Id, ws2Owner.Id);

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

        await Task.WhenAll(task1, task2);

        // Assert: Exactly one operation succeeded and exactly one threw LastOwnerCannotLeave
        var exceptions = new[] { ex1, ex2 }.Where(e => e != null).ToList();
        exceptions.Count.ShouldBe(1);

        var businessEx = exceptions[0].ShouldBeOfType<BusinessException>();
        businessEx.Code.ShouldBe(TrackerDomainErrorCodes.LastOwnerCannotLeave);

        // Assert: Persisted active owners in WS1 is exactly 1 (final persisted count >= 1)
        var finalWs1Owners = await CountActiveOwnersInDbAsync(ws1Id);
        finalWs1Owners.ShouldBe(1);

        // Assert: Winner was removed and had project memberships removed;
        // Loser remained active Owner and project membership is intact
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
                    // Owner 1 won (left)
                    mem1.Status.ShouldBe(WorkspaceMemberStatus.Removed);
                    projMembers1.ShouldBeEmpty();

                    // Owner 2 lost (kept as last owner)
                    mem2.Status.ShouldBe(WorkspaceMemberStatus.Active);
                    mem2.Role.ShouldBe(WorkspaceRoles.Owner);
                    projMembers2.Count.ShouldBe(1);
                }
                else
                {
                    // Owner 2 won (left)
                    mem2.Status.ShouldBe(WorkspaceMemberStatus.Removed);
                    projMembers2.ShouldBeEmpty();

                    // Owner 1 lost (kept as last owner)
                    mem1.Status.ShouldBe(WorkspaceMemberStatus.Active);
                    mem1.Role.ShouldBe(WorkspaceRoles.Owner);
                    projMembers1.Count.ShouldBe(1);
                }

                // Assert: WS2 is completely unaffected (no unintended membership/project cleanup)
                var ws2OwnerMem = await memRepo.FirstAsync(m => m.WorkspaceId == ws2Id && m.UserId == ws2Owner.Id);
                ws2OwnerMem.Status.ShouldBe(WorkspaceMemberStatus.Active);
                ws2OwnerMem.Role.ShouldBe(WorkspaceRoles.Owner);

                var ws2ProjMembers = await projMemRepo.GetListAsync(pm => pm.WorkspaceId == ws2Id && pm.UserId == ws2Owner.Id);
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
        // Exactly one succeeds, and the other is rejected with LastOwnerCannotLeave.
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

        await Task.WhenAll(taskA, taskB);

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
    public async Task Concurrent_Mixed_Owner_Leave_And_Demotion_Preserves_At_Least_One_Active_Owner()
    {
        // Passing-after regression:
        // Owner A leaves while Owner B concurrently demotes Owner A to Manager.
        // Under per-workspace serialization and transactional lock scoping:
        // Either Leave commits first (so Demote throws InvalidUser) OR
        // Demote commits first (Owner A becomes Manager, Leave removes Manager A, Owner B remains Owner).
        // In all cases, active owners count in the workspace NEVER drops below 1.

        var wsId = _guidGenerator.Create();
        var ownerA = await CreateTestUserAsync("mixed_a");
        var ownerB = await CreateTestUserAsync("mixed_b");

        await CreateWorkspaceWithOwnerAsync(wsId, "WS_MixedLeaveDemote", ownerA.Id);
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

                using (principalAccessor.Change(CreatePrincipal(ownerA.Id)))
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

        await Task.WhenAll(taskLeave, taskDemote);

        // In both interleavings, Owner B remains an active Owner
        var finalOwners = await CountActiveOwnersInDbAsync(wsId);
        finalOwners.ShouldBe(1);

        using (var verifyScope = ServiceProvider.CreateScope())
        {
            var uowManager = verifyScope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
            using var uow = uowManager.Begin(requiresNew: true);
            var memRepo = verifyScope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>();
            var memB = await memRepo.FirstAsync(m => m.WorkspaceId == wsId && m.UserId == ownerB.Id);
            memB.Status.ShouldBe(WorkspaceMemberStatus.Active);
            memB.Role.ShouldBe(WorkspaceRoles.Owner);
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

        await Task.WhenAll(task1, task2);

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
