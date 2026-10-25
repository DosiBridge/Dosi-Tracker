using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.BlobStoring;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.SettingManagement;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-DAT-01: Owner-managed workspace capture retention policy lifecycle test suite.
/// Uses hosted ASP.NET Core TestServer MVC pipeline, disposable project-local SQLite,
/// real WorkspaceMembershipManager aggregate authorization, real persisted workspace-keyed settings,
/// and instrumented in-memory ScriptedBlobProvider.
/// </summary>
public class WorkspaceCaptureRetentionPolicyHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb21");
    private static readonly Guid ManagerA = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb22");
    private static readonly Guid MemberA = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb23");
    private static readonly Guid InactiveMemberA = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb24");
    private static readonly Guid UserB = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb25");
    private static readonly Guid DualUser = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb26");

    private readonly ScriptedBlobProvider _blobProvider = new();
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;
    private readonly WorkspaceSettingsManager _settingsManager;
    private readonly ISettingManager _settingManager;
    private IDataFilter DataFilter => GetRequiredService<IDataFilter>();

    public WorkspaceCaptureRetentionPolicyHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
        _settingsManager = GetRequiredService<WorkspaceSettingsManager>();
        _settingManager = GetRequiredService<ISettingManager>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var db = ServiceProvider.GetRequiredService<TrackerDbContext>();
        var conn = db.Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
        {
            options.Configure(ctx =>
            {
                ctx.DbContextOptions.UseSqlite(conn);
            });
        });
        services.AddAlwaysDisableUnitOfWorkTransaction();
        services.AddControllers().AddApplicationPart(typeof(WorkspacesController).Assembly);

        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB)> SeedPolicyFixtureAsync()
    {
        async Task EnsureUserAsync(Guid id, string userName, string email)
        {
            if (await _userManager.FindByIdAsync(id.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(id, userName, email));
            }
        }

        await EnsureUserAsync(OwnerA, "ownerA", "ownerA@dosi.test");
        await EnsureUserAsync(ManagerA, "managerA", "managerA@dosi.test");
        await EnsureUserAsync(MemberA, "memberA", "memberA@dosi.test");
        await EnsureUserAsync(InactiveMemberA, "inactiveA", "inactiveA@dosi.test");
        await EnsureUserAsync(UserB, "userB", "userB@dosi.test");
        await EnsureUserAsync(DualUser, "dualUser", "dualUser@dosi.test");

        var wsA = await ServiceProvider.CreateWorkspaceAsync("POLICY-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("POLICY-WS-B");

        // Workspace A memberships
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerA, WorkspaceRoles.Owner, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(ManagerA, WorkspaceRoles.Manager, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(MemberA, WorkspaceRoles.Member, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Owner, wsA);

        // Inactive member in Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var inactive = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveMemberA, WorkspaceRoles.Member, DateTime.UtcNow);
            inactive.Remove();
            await _memberRepository.InsertAsync(inactive, autoSave: true);
        });

        // Workspace B memberships
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsB);

        return (wsA, wsB);
    }

    private async Task<(Guid ProjectId, Guid ActivityId)> SeedActivityAsync(Guid workspaceId, DateTime startedAt)
    {
        var projId = _guidGenerator.Create();
        var actId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _projectRepository.InsertAsync(new Project(projId, workspaceId, $"Proj-{projId:N}"), autoSave: true);

                var activity = new Activity(
                    id: actId,
                    workspaceId: workspaceId,
                    userId: OwnerA,
                    projectId: projId,
                    clientActivityId: _guidGenerator.Create(),
                    startedAt: startedAt,
                    endedAt: startedAt.AddMinutes(10))
                {
                    Productivity = 90,
                    MouseClicks = 200,
                    KeyboardHits = 450,
                    ActiveWindowsJson = "[\"Code.exe\"]",
                    RunningProgramsJson = "[\"devenv.exe\"]",
                    TimelineJson = "[{\"m\":0,\"k\":45}]"
                };

                await _activityRepository.InsertAsync(activity, autoSave: true);
            }
        });

        return (projId, actId);
    }

    private async Task<Screenshot> SeedScreenshotAsync(Guid workspaceId, Guid activityId, DateTime capturedAt, string kind = Screenshot.ScreenKind)
    {
        var scId = _guidGenerator.Create();
        var storageUrl = $"activities/{activityId:D}/{kind}.jpg";
        var sc = new Screenshot(
            id: scId,
            workspaceId: workspaceId,
            activityId: activityId,
            storageUrl: storageUrl,
            capturedAt: capturedAt,
            sizeBytes: 1024,
            contentType: "image/jpeg",
            kind: kind);

        await WithUnitOfWorkAsync(async () =>
        {
            using (DataFilter.Disable<IWorkspaceScoped>())
            {
                await _screenshotRepository.InsertAsync(sc, autoSave: true);
            }
        });

        _blobProvider.Blobs[storageUrl] = new byte[] { 0xAA, 0xBB, 0xCC, 0xDD };
        return sc;
    }

    [Fact]
    public async Task Real_Hosted_Route_Get_Returns_Safe_DefaultOff_Policy_For_Active_Members()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // 1. Owner can read default policy (safe default-off, bounded 30 days)
        var reqOwner = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA);
        var respOwner = await client.SendAsync(reqOwner);
        respOwner.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docOwner = JsonDocument.Parse(await respOwner.Content.ReadAsStringAsync());
        docOwner.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docOwner.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);

        // 2. Member can read default policy
        var reqMember = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", MemberA, wsA);
        var respMember = await client.SendAsync(reqMember);
        respMember.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docMember = JsonDocument.Parse(await respMember.Content.ReadAsStringAsync());
        docMember.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docMember.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);

        // 3. Manager can read default policy
        var reqManager = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", ManagerA, wsA);
        var respManager = await client.SendAsync(reqManager);
        respManager.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docManager = JsonDocument.Parse(await respManager.Content.ReadAsStringAsync());
        docManager.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docManager.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
    }

    [Fact]
    public async Task Unauthenticated_Requests_Are_Rejected_With_401()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // Anonymous GET
        var reqGet = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", userId: null, workspaceId: null);
        var respGet = await client.SendAsync(reqGet);
        respGet.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);

        // Anonymous PUT
        var payload = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 60 };
        var reqPut = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", userId: null, workspaceId: null, jsonBody: payload);
        var respPut = await client.SendAsync(reqPut);
        respPut.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
    }

    [Fact]
    public async Task Cross_Workspace_Isolation_UserA_Cannot_Read_Or_Write_WorkspaceB()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // Owner of A (non-member of B) attempts GET B
        var reqGetOwner = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsB}/capture-retention", OwnerA, wsB);
        var respGetOwner = await client.SendAsync(reqGetOwner);
        respGetOwner.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Owner of A attempts PUT B
        var payload = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 60 };
        var reqPutOwner = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsB}/capture-retention", OwnerA, wsB, jsonBody: payload);
        var respPutOwner = await client.SendAsync(reqPutOwner);
        respPutOwner.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Member of A attempts GET B
        var reqGetMember = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsB}/capture-retention", MemberA, wsB);
        var respGetMember = await client.SendAsync(reqGetMember);
        respGetMember.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Member of A attempts PUT B
        var reqPutMember = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsB}/capture-retention", MemberA, wsB, jsonBody: payload);
        var respPutMember = await client.SendAsync(reqPutMember);
        respPutMember.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
    }

    [Fact]
    public async Task Non_Owner_In_WorkspaceA_Cannot_Change_Policy()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        var payload = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 60 };

        // Member attempts PUT A
        var reqMember = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", MemberA, wsA, jsonBody: payload);
        var respMember = await client.SendAsync(reqMember);
        respMember.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Manager attempts PUT A
        var reqManager = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", ManagerA, wsA, jsonBody: payload);
        var respManager = await client.SendAsync(reqManager);
        respManager.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Verify Workspace A policy remains default
        var reqVerify = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA);
        var respVerify = await client.SendAsync(reqVerify);
        respVerify.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docVerify = JsonDocument.Parse(await respVerify.Content.ReadAsStringAsync());
        docVerify.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docVerify.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
    }

    [Fact]
    public async Task Dual_Member_OwnerA_MemberB_Can_Read_Both_And_Mutate_A_But_Cannot_Mutate_B()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // DualUser reads A -> 200 OK
        var reqReadA = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", DualUser, wsA);
        var respReadA = await client.SendAsync(reqReadA);
        respReadA.StatusCode.ShouldBe(HttpStatusCode.OK);

        // DualUser reads B -> 200 OK
        var reqReadB = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsB}/capture-retention", DualUser, wsB);
        var respReadB = await client.SendAsync(reqReadB);
        respReadB.StatusCode.ShouldBe(HttpStatusCode.OK);

        // DualUser updates A (as Owner) -> 200 OK
        var payloadA = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 90 };
        var reqMutateA = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", DualUser, wsA, jsonBody: payloadA);
        var respMutateA = await client.SendAsync(reqMutateA);
        respMutateA.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docA = JsonDocument.Parse(await respMutateA.Content.ReadAsStringAsync());
        docA.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeTrue();
        docA.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(90);

        // DualUser attempts to update B (as Member only) -> 403 Forbidden!
        var payloadB = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 90 };
        var reqMutateB = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsB}/capture-retention", DualUser, wsB, jsonBody: payloadB);
        var respMutateB = await client.SendAsync(reqMutateB);
        respMutateB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Verify B remains default
        var reqVerifyB = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsB}/capture-retention", UserB, wsB);
        var respVerifyB = await client.SendAsync(reqVerifyB);
        var docVerifyB = JsonDocument.Parse(await respVerifyB.Content.ReadAsStringAsync());
        docVerifyB.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docVerifyB.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
    }

    [Fact]
    public async Task OwnerA_Can_Update_Policy_And_Subsequent_Read_Persists_While_WorkspaceB_Remains_Unchanged()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // Owner A updates policy for Workspace A
        var updatePayload = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 60 };
        var reqPut = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA, jsonBody: updatePayload);
        var respPut = await client.SendAsync(reqPut);
        respPut.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docPut = JsonDocument.Parse(await respPut.Content.ReadAsStringAsync());
        docPut.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeTrue();
        docPut.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(60);

        // Subsequent read by Member A confirms persisted policy
        var reqGetMember = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", MemberA, wsA);
        var respGetMember = await client.SendAsync(reqGetMember);
        respGetMember.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docGetMember = JsonDocument.Parse(await respGetMember.Content.ReadAsStringAsync());
        docGetMember.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeTrue();
        docGetMember.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(60);

        // Workspace B settings sentinel remains completely unchanged
        var reqGetB = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsB}/capture-retention", UserB, wsB);
        var respGetB = await client.SendAsync(reqGetB);
        respGetB.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docGetB = JsonDocument.Parse(await respGetB.Content.ReadAsStringAsync());
        docGetB.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docGetB.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
    }

    [Fact]
    public async Task Inactive_Or_Removed_Member_Is_Rejected_With_403()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // Inactive member GET A -> 403 Forbidden
        var reqGet = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", InactiveMemberA, wsA);
        var respGet = await client.SendAsync(reqGet);
        respGet.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Inactive member PUT A -> 403 Forbidden
        var payload = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 60 };
        var reqPut = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", InactiveMemberA, wsA, jsonBody: payload);
        var respPut = await client.SendAsync(reqPut);
        respPut.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
    }

    [Fact]
    public async Task Invalid_Inputs_Cause_Zero_Policy_Changes()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        // Test 0 days (below minimum of 1)
        var payloadZero = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 0 };
        var reqZero = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA, jsonBody: payloadZero);
        var respZero = await client.SendAsync(reqZero);
        respZero.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // Test negative days
        var payloadNeg = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = -10 };
        var reqNeg = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA, jsonBody: payloadNeg);
        var respNeg = await client.SendAsync(reqNeg);
        respNeg.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // Test days above maximum of 730
        var payloadMax = new WorkspaceCaptureRetentionDto { IsEnabled = true, RetentionDays = 1000 };
        var reqMax = CreateRequest(HttpMethod.Put, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA, jsonBody: payloadMax);
        var respMax = await client.SendAsync(reqMax);
        respMax.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // Subsequent GET confirms zero policy changes
        var reqVerify = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsA}/capture-retention", OwnerA, wsA);
        var respVerify = await client.SendAsync(reqVerify);
        respVerify.StatusCode.ShouldBe(HttpStatusCode.OK);

        var docVerify = JsonDocument.Parse(await respVerify.Content.ReadAsStringAsync());
        docVerify.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
        docVerify.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
    }

    [Fact]
    public async Task Worker_MasterDisabled_Makes_Zero_Cleanup_Calls_Even_When_Workspace_OptedIn()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var (_, actA) = await SeedActivityAsync(wsA, _clock.Now.AddDays(-50));
        var scA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-45));

        // Opt in Workspace A to 30 days
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsA, isEnabled: true, retentionDays: 30);
        });

        // Master options: IsEnabled = false
        var disabledOptions = Options.Create(new CaptureRetentionOptions { IsEnabled = false });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, disabledOptions);

        // Sweep
        var result = await worker.SweepAsync(ServiceProvider);

        // Assert: 0 calls, 0 deleted, skipped by config
        result.Skipped.ShouldBeTrue();
        result.WorkspacesSwept.ShouldBe(0);
        result.TotalDeleted.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scA.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scA.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Worker_MasterEnabled_Skips_Unset_Or_OptedOut_WorkspaceB_And_Cleans_OptedIn_WorkspaceA()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var (_, actA) = await SeedActivityAsync(wsA, _clock.Now.AddDays(-50));
        var (_, actB) = await SeedActivityAsync(wsB, _clock.Now.AddDays(-50));

        var scA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-40));
        var scB = await SeedScreenshotAsync(wsB, actB, _clock.Now.AddDays(-40));

        // Explicitly opt in Workspace A (30 days)
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsA, isEnabled: true, retentionDays: 30);
        });

        // Workspace B is left UNSET (default-off: IsEnabled = false)
        var policyB = await _settingsManager.GetCaptureRetentionPolicyAsync(wsB);
        policyB.IsEnabled.ShouldBeFalse();

        // Worker with master enabled
        var enabledOptions = Options.Create(new CaptureRetentionOptions
        {
            IsEnabled = true,
            BatchSize = 100
        });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, enabledOptions);

        // Sweep
        var result = await worker.SweepAsync(ServiceProvider);
        result.Skipped.ShouldBeFalse();

        // Workspace A expired capture was cleaned
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scA.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scA.StorageUrl).ShouldBeFalse();

        // Workspace B expired capture stays 100% UNTOUCHED
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scB.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scB.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Worker_OptedIn_Workspace_Own_Duration_Controls_Selection_With_Boundary_Sentinels()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var (_, actA) = await SeedActivityAsync(wsA, _clock.Now.AddDays(-80));
        var (_, actB) = await SeedActivityAsync(wsB, _clock.Now.AddDays(-80));

        // Workspace A configured with 60 days duration (not global 30 days)
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsA, isEnabled: true, retentionDays: 60);
        });

        // Seed captures in Workspace A:
        // 1. Expired: 70 days old (> 60 days) -> should be deleted
        var scExpiredA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-70), kind: Screenshot.ScreenKind);
        // 2. Boundary sentinel: 45 days old (> 30 days, but < 60 days) -> MUST BE PRESERVED!
        var scBoundaryA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-45), kind: Screenshot.ThumbKind);
        // 3. Fresh: 10 days old -> MUST BE PRESERVED!
        var scFreshA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-10), kind: Screenshot.WebcamKind);

        // Foreign capture in Workspace B (70 days old, opted out) -> MUST BE PRESERVED!
        var scForeignB = await SeedScreenshotAsync(wsB, actB, _clock.Now.AddDays(-70), kind: Screenshot.ScreenKind);

        var enabledOptions = Options.Create(new CaptureRetentionOptions
        {
            IsEnabled = true,
            BatchSize = 100
        });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, enabledOptions);

        // Sweep
        var result = await worker.SweepAsync(ServiceProvider);

        // Assertions:
        // Expired in A was cleaned
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scExpiredA.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scExpiredA.StorageUrl).ShouldBeFalse();

        // Boundary sentinel in A (45 days old) is preserved!
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scBoundaryA.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scBoundaryA.StorageUrl).ShouldBeTrue();

        // Fresh capture in A is preserved!
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scFreshA.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scFreshA.StorageUrl).ShouldBeTrue();

        // Activity and time summaries in A remain completely untouched
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            var act = await _activityRepository.FindAsync(actA);
            act.ShouldNotBeNull();
            act.Productivity.ShouldBe(90);
            act.MouseClicks.ShouldBe(200);
            act.KeyboardHits.ShouldBe(450);
            act.ActiveWindowsJson.ShouldBe("[\"Code.exe\"]");
            act.RunningProgramsJson.ShouldBe("[\"devenv.exe\"]");
            act.TimelineJson.ShouldBe("[{\"m\":0,\"k\":45}]");
        }

        // Foreign capture in Workspace B is untouched
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scForeignB.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scForeignB.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Worker_Disabling_Policy_Stops_Later_Sweeps_And_Preserves_Foreign_Captures_And_Summaries()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var (_, actA) = await SeedActivityAsync(wsA, _clock.Now.AddDays(-60));
        var (_, actB) = await SeedActivityAsync(wsB, _clock.Now.AddDays(-60));

        // Workspace A explicitly opted out (IsEnabled = false)
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsA, isEnabled: false, retentionDays: 30);
        });

        var scA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-50));
        var scB = await SeedScreenshotAsync(wsB, actB, _clock.Now.AddDays(-50));

        var enabledOptions = Options.Create(new CaptureRetentionOptions
        {
            IsEnabled = true,
            BatchSize = 100
        });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, enabledOptions);

        // Sweep
        var result = await worker.SweepAsync(ServiceProvider);

        // Both workspaces are opted out / unset -> 0 deleted
        result.TotalDeleted.ShouldBe(0);

        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scA.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(scB.Id)).ShouldNotBeNull();

            var actAFromDb = await _activityRepository.FindAsync(actA);
            actAFromDb.ShouldNotBeNull();
            actAFromDb.Productivity.ShouldBe(90);
        }
        _blobProvider.Blobs.ContainsKey(scA.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(scB.StorageUrl).ShouldBeTrue();
    }

    [Fact]
    public async Task Persisted_Malformed_Or_Missing_RetentionDays_With_Enabled_True_Fails_Closed_In_Manager_And_Hosted_Read()
    {
        var (wsA, _) = await SeedPolicyFixtureAsync();
        var client = CreateHttpClient();

        var malformedOrMissingCases = new (string? RawDays, string Description)[]
        {
            (null, "missing/null days"),
            ("", "empty string days"),
            ("   ", "whitespace days"),
            ("nonnumeric", "non-numeric string days"),
            ("corrupt_value", "corrupt text days"),
            ("999999999999999999999999999", "integer overflow days"),
            ("0", "out of bounds low (0 days)"),
            ("-10", "out of bounds negative days"),
            ("731", "out of bounds high (731 days)"),
            ("10000", "out of bounds high (10000 days)")
        };

        foreach (var (rawDays, desc) in malformedOrMissingCases)
        {
            var wsCase = await ServiceProvider.CreateWorkspaceAsync($"MALFORMED-{Guid.NewGuid():N}");
            await ServiceProvider.AddWorkspaceMemberAsync(OwnerA, WorkspaceRoles.Owner, wsCase);

            // Persist IsEnabled = true with the malformed/missing rawDays setting on provider "W"
            await WithUnitOfWorkAsync(async () =>
            {
                await _settingManager.SetAsync(
                    TrackerSettings.Workspace.CaptureRetention.IsEnabled,
                    "true",
                    WorkspaceSettingValueProvider.ProviderName,
                    wsCase.ToString());

                if (rawDays != null)
                {
                    await _settingManager.SetAsync(
                        TrackerSettings.Workspace.CaptureRetention.RetentionDays,
                        rawDays,
                        WorkspaceSettingValueProvider.ProviderName,
                        wsCase.ToString());
                }
            });

            // 1. Production manager must fail closed: IsEnabled = false, RetentionDays = 30
            var policy = await _settingsManager.GetCaptureRetentionPolicyAsync(wsCase);
            policy.IsEnabled.ShouldBeFalse($"Expected IsEnabled to fail closed (false) for {desc}, but got {policy.IsEnabled}");
            policy.RetentionDays.ShouldBe(30, $"Expected RetentionDays to fail closed to safe default (30) for {desc}, but got {policy.RetentionDays}");

            // 2. Authenticated hosted GET route must reflect fail-closed disabled policy
            var reqGet = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsCase}/capture-retention", OwnerA, wsCase);
            var respGet = await client.SendAsync(reqGet);
            respGet.StatusCode.ShouldBe(HttpStatusCode.OK);

            var docGet = JsonDocument.Parse(await respGet.Content.ReadAsStringAsync());
            docGet.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse($"Hosted GET must report disabled for {desc}");
            docGet.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(30);
        }

        // Control: Explicitly valid bounded policy remains operative
        var wsValid = await ServiceProvider.CreateWorkspaceAsync("VALID-WS");
        await ServiceProvider.AddWorkspaceMemberAsync(OwnerA, WorkspaceRoles.Owner, wsValid);

        await WithUnitOfWorkAsync(async () =>
        {
            await _settingManager.SetAsync(
                TrackerSettings.Workspace.CaptureRetention.IsEnabled,
                "true",
                WorkspaceSettingValueProvider.ProviderName,
                wsValid.ToString());

            await _settingManager.SetAsync(
                TrackerSettings.Workspace.CaptureRetention.RetentionDays,
                "45",
                WorkspaceSettingValueProvider.ProviderName,
                wsValid.ToString());
        });

        var validPolicy = await _settingsManager.GetCaptureRetentionPolicyAsync(wsValid);
        validPolicy.IsEnabled.ShouldBeTrue();
        validPolicy.RetentionDays.ShouldBe(45);

        var reqValid = CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{wsValid}/capture-retention", OwnerA, wsValid);
        var respValid = await client.SendAsync(reqValid);
        respValid.StatusCode.ShouldBe(HttpStatusCode.OK);
        var docValid = JsonDocument.Parse(await respValid.Content.ReadAsStringAsync());
        docValid.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeTrue();
        docValid.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(45);
    }

    [Fact]
    public async Task Worker_Sweep_With_Malformed_RetentionDays_Makes_Zero_Deletions_While_Valid_Control_Cleans_In_Same_Sweep()
    {
        var (wsA, wsB) = await SeedPolicyFixtureAsync();
        var wsC = await ServiceProvider.CreateWorkspaceAsync("POLICY-WS-C");

        // Seed captures older than 30 days in all 3 workspaces:
        var (_, actA) = await SeedActivityAsync(wsA, _clock.Now.AddDays(-60));
        var (_, actB) = await SeedActivityAsync(wsB, _clock.Now.AddDays(-60));
        var (_, actC) = await SeedActivityAsync(wsC, _clock.Now.AddDays(-60));

        var scOldA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-45), kind: Screenshot.ScreenKind);
        var scFreshA = await SeedScreenshotAsync(wsA, actA, _clock.Now.AddDays(-5), kind: Screenshot.WebcamKind);

        var scOldB = await SeedScreenshotAsync(wsB, actB, _clock.Now.AddDays(-45), kind: Screenshot.ScreenKind);
        var scOldC = await SeedScreenshotAsync(wsC, actC, _clock.Now.AddDays(-45), kind: Screenshot.ScreenKind);

        // Workspace A: Persisted IsEnabled = true, but RetentionDays is corrupt/non-numeric ("corrupted_duration")
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingManager.SetAsync(
                TrackerSettings.Workspace.CaptureRetention.IsEnabled,
                "true",
                WorkspaceSettingValueProvider.ProviderName,
                wsA.ToString());

            await _settingManager.SetAsync(
                TrackerSettings.Workspace.CaptureRetention.RetentionDays,
                "corrupted_duration",
                WorkspaceSettingValueProvider.ProviderName,
                wsA.ToString());
        });

        // Workspace B: Control workspace explicitly opted in with valid 30 days
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsB, isEnabled: true, retentionDays: 30);
        });

        // Workspace C: Opted-out sentinel (IsEnabled = false)
        await WithUnitOfWorkAsync(async () =>
        {
            await _settingsManager.SetCaptureRetentionPolicyAsync(wsC, isEnabled: false, retentionDays: 30);
        });

        // Master-enabled worker
        var enabledOptions = Options.Create(new CaptureRetentionOptions
        {
            IsEnabled = true,
            BatchSize = 100
        });
        var timer = GetRequiredService<AbpAsyncTimer>();
        var scopeFactory = GetRequiredService<IServiceScopeFactory>();
        var worker = new CaptureRetentionWorker(timer, scopeFactory, enabledOptions);

        // Run isolated sweep
        var result = await worker.SweepAsync(ServiceProvider);
        result.Skipped.ShouldBeFalse();

        // Workspace A (corrupted days) MUST HAVE ZERO DELETIONS (failed closed):
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scOldA.Id)).ShouldNotBeNull();
            (await _screenshotRepository.FindAsync(scFreshA.Id)).ShouldNotBeNull();

            // Activity summary in Workspace A preserved
            var act = await _activityRepository.FindAsync(actA);
            act.ShouldNotBeNull();
            act.Productivity.ShouldBe(90);
        }
        _blobProvider.Blobs.ContainsKey(scOldA.StorageUrl).ShouldBeTrue();
        _blobProvider.Blobs.ContainsKey(scFreshA.StorageUrl).ShouldBeTrue();

        // Workspace B (valid control) MUST BE CLEANED (proves worker ran):
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scOldB.Id)).ShouldBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scOldB.StorageUrl).ShouldBeFalse();

        // Workspace C (opted-out sentinel) MUST BE PRESERVED:
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            (await _screenshotRepository.FindAsync(scOldC.Id)).ShouldNotBeNull();
        }
        _blobProvider.Blobs.ContainsKey(scOldC.StorageUrl).ShouldBeTrue();
    }
}
