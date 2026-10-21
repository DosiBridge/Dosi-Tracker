using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Devices;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.BlobStoring;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Security.Claims;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>
/// Regressions for workspace-scoped self-service device revocation:
/// 1. Own device revoke and repeated revoke (idempotent 200 OK).
/// 2. Post-revoke heartbeat denied (403 Tracker:DeviceRevoked) with persisted sentinels.
/// 3. Post-revoke activity ingest denied (403 Tracker:DeviceRevoked) with zero blob/screenshot side-effects,
///    verified against an instrumented project-local in-memory ScriptedBlobProvider and allowed-upload control.
/// 4. Idempotent replay of previously stored block for revoked device succeeds.
/// 5. Historical records retained and current live presence projection excluded.
/// 6. Legitimate different device for caller in A continues to work.
/// 7. Same user/device ID in Workspace B unaffected (strictly workspace-scoped).
/// 8. Another member's device in A and real-target foreign workspace revocation denied without creating tombstones,
///    asserting exact seeded state preservation in target workspace.
/// 9. Stale presence cleanup cannot remove persistent revocation record.
/// 10. Malformed/unobserved device targets rejected (400 Tracker:DeviceNotObserved) without creating tombstones.
/// 11. Status inspection and bounded listing contracts work as expected.
/// 12. Legacy missing-DeviceId activity behavior preserved.
/// </summary>
public class WorkspaceDeviceRevocationHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid UserA = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid ForeignUser = Guid.Parse("6a701e62-0953-4dd3-910b-dc6cc93ccb03");

    private readonly ScriptedBlobProvider _blobProvider = new();

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceDeviceRevocationHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _projectMemberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _presenceRepository = GetRequiredService<IRepository<AgentPresence, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _revocationRepository = GetRequiredService<IRepository<DeviceRevocation, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
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

        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private new static HttpRequestMessage CreateRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        Guid? workspaceId,
        object? jsonBody = null)
    {
        return WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA, Guid ProjectB)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();
        var projA = _guidGenerator.Create();
        var projB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(UserA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserA, "userA_dev", "userA_dev@test.com"));
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ColleagueA, "colleagueA_dev", "colleagueA_dev@test.com"));
            }
            if (await _userManager.FindByIdAsync(ForeignUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ForeignUser, "foreign_dev", "foreign_dev@test.com"));
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace Alpha"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace Beta"), autoSave: true);

            var now = DateTime.UtcNow;
            // Workspace A: UserA (Owner), ColleagueA (Member)
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, UserA, WorkspaceRoles.Owner, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, now), autoSave: true);

            // Workspace B: UserA (Member), ForeignUser (Owner)
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, UserA, WorkspaceRoles.Member, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, ForeignUser, WorkspaceRoles.Owner, now), autoSave: true);

            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                await _projectRepository.InsertAsync(new Project(projA, wsA, "Project Alpha"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA, UserA, "Developer"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA, ColleagueA, "Developer"), autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                await _projectRepository.InsertAsync(new Project(projB, wsB, "Project Beta"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB, UserA, "Developer"), autoSave: true);
            }
        });

        return (wsA, wsB, projA, projB);
    }

    [Fact]
    public async Task Own_Device_Revocation_And_Repeated_Revoke_Are_Idempotent_And_Persisted()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var deviceId = _guidGenerator.Create();

        // 1. Establish observed device for UserA in wsA via heartbeat
        var heartbeatReq = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new
        {
            deviceId = deviceId,
            status = "tracking",
            deviceName = "MY-LAPTOP"
        });
        var heartbeatResp = await client.SendAsync(heartbeatReq);
        heartbeatResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Revoke own device
        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new
        {
            deviceId = deviceId,
            reason = "Stolen laptop"
        });
        var revokeResp = await client.SendAsync(revokeReq);
        revokeResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var revokeJson = await revokeResp.Content.ReadAsStringAsync();
        using (var doc = JsonDocument.Parse(revokeJson))
        {
            doc.RootElement.GetProperty("deviceId").GetString().ShouldBe(deviceId.ToString());
            doc.RootElement.GetProperty("reason").GetString().ShouldBe("Stolen laptop");
            doc.RootElement.GetProperty("workspaceId").GetString().ShouldBe(wsA.ToString());
            doc.RootElement.GetProperty("userId").GetString().ShouldBe(UserA.ToString());
        }

        // Verify persisted in AppDeviceRevocations
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var count = await _revocationRepository.CountAsync(r => r.WorkspaceId == wsA && r.UserId == UserA && r.DeviceId == deviceId);
                count.ShouldBe(1);
            }
        });

        // 3. Repeated revoke is idempotent (200 OK without creating duplicate rows)
        var repeatReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new
        {
            deviceId = deviceId,
            reason = "Another reason attempt"
        });
        var repeatResp = await client.SendAsync(repeatReq);
        repeatResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var count = await _revocationRepository.CountAsync(r => r.WorkspaceId == wsA && r.UserId == UserA && r.DeviceId == deviceId);
                count.ShouldBe(1);
            }
        });
    }

    [Fact]
    public async Task Post_Revoke_Heartbeat_And_Activity_Ingestion_Are_Denied_With_Zero_Blob_Effects()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var allowedDeviceId = _guidGenerator.Create();
        var revokedDeviceId = _guidGenerator.Create();

        // 1. Preseed a sentinel blob directly into the in-memory provider
        var sentinelKey = "sentinels/preseeded-blob.bin";
        var sentinelBytes = new byte[] { 0xDE, 0xAD, 0xBE, 0xEF, 0x42 };
        _blobProvider.Blobs[sentinelKey] = (byte[])sentinelBytes.Clone();
        var initialSaveCount = _blobProvider.Blobs.Count; // 1

        // 2. Control upload on an active, non-revoked device with valid synthetic capture payload.
        // Demonstrates that the test fixture's blob provider instrumentation truthfully observes actual production blob writes!
        var jpeg = Convert.ToBase64String(new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02, 0xFF, 0xD9 });
        var controlBlockTime = DateTime.UtcNow.AddMinutes(-30);
        var controlActId = _guidGenerator.Create();
        var controlUploadReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = controlActId,
            startedAt = controlBlockTime,
            endedAt = controlBlockTime.AddMinutes(10),
            productivity = 90,
            deviceId = allowedDeviceId,
            screenshotPngBase64 = jpeg,
            screenshotThumbJpgBase64 = jpeg
        });
        var controlResp = await client.SendAsync(controlUploadReq);
        controlResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // Verify that the control upload triggered real production blob writes in our instrumented provider
        _blobProvider.Blobs.Count.ShouldBeGreaterThan(initialSaveCount);
        _blobProvider.Blobs.Keys.Any(k => k.StartsWith("activities/")).ShouldBeTrue();

        // 3. Take a strict snapshot of all blob keys and byte arrays, delete count, and SQL counts before the revoked attempt
        var blobSnapshotBeforeDenied = _blobProvider.Blobs.ToDictionary(k => k.Key, v => v.Value.ToArray());
        var blobCountBeforeDenied = _blobProvider.Blobs.Count;
        var deleteCountBeforeDenied = _blobProvider.DeleteCount;

        long sqlScreenshotsBeforeDenied = 0;
        long sqlActivitiesBeforeDenied = 0;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                sqlScreenshotsBeforeDenied = await _screenshotRepository.GetCountAsync();
                sqlActivitiesBeforeDenied = await _activityRepository.GetCountAsync();
            }
        });

        // 4. Observe revokedDeviceId in wsA, then revoke it
        var initHeartbeat = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new
        {
            deviceId = revokedDeviceId,
            status = "tracking"
        });
        (await client.SendAsync(initHeartbeat)).StatusCode.ShouldBe(HttpStatusCode.OK);

        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new
        {
            deviceId = revokedDeviceId,
            reason = "Compromised machine"
        });
        (await client.SendAsync(revokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 5. Subsequent heartbeat from revoked device is DENIED (403 Forbidden with Tracker:DeviceRevoked)
        var deniedHeartbeat = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new
        {
            deviceId = revokedDeviceId,
            status = "tracking"
        });
        var hbResp = await client.SendAsync(deniedHeartbeat);
        hbResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var hbBody = await hbResp.Content.ReadAsStringAsync();
        hbBody.ShouldContain("Tracker:DeviceRevoked");

        // 6. Subsequent activity upload from revoked device is DENIED (403 Forbidden with Tracker:DeviceRevoked)
        var deniedBlockTime = DateTime.UtcNow.AddMinutes(-10);
        var deniedActId = _guidGenerator.Create();
        var deniedActivityReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = deniedActId,
            startedAt = deniedBlockTime,
            endedAt = deniedBlockTime.AddMinutes(10),
            productivity = 80,
            deviceId = revokedDeviceId,
            screenshotPngBase64 = jpeg,
            screenshotThumbJpgBase64 = jpeg
        });

        var actResp = await client.SendAsync(deniedActivityReq);
        actResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var actBody = await actResp.Content.ReadAsStringAsync();
        actBody.ShouldContain("Tracker:DeviceRevoked");

        // 7. Verify ZERO blob writes, ZERO blob deletes, and exact preserved preseeded/control blob keys & bytes
        _blobProvider.Blobs.Count.ShouldBe(blobCountBeforeDenied);
        _blobProvider.DeleteCount.ShouldBe(deleteCountBeforeDenied);

        foreach (var (key, expectedBytes) in blobSnapshotBeforeDenied)
        {
            _blobProvider.Blobs.TryGetValue(key, out var currentBytes).ShouldBeTrue();
            currentBytes.ShouldNotBeNull();
            currentBytes.SequenceEqual(expectedBytes).ShouldBeTrue();
        }

        // Prove no orphan blob was created for the denied activity
        _blobProvider.Blobs.Keys.Any(k => k.Contains(deniedActId.ToString())).ShouldBeFalse();

        // 8. Verify SQL row counts (activities and screenshots) remain completely unchanged
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var sqlScreenshotsAfter = await _screenshotRepository.GetCountAsync();
                var sqlActivitiesAfter = await _activityRepository.GetCountAsync();
                sqlScreenshotsAfter.ShouldBe(sqlScreenshotsBeforeDenied);
                sqlActivitiesAfter.ShouldBe(sqlActivitiesBeforeDenied);
            }
        });
    }

    [Fact]
    public async Task Idempotent_Replay_Of_Pre_Revocation_Block_Succeeds_While_New_Block_Is_Denied()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var deviceId = _guidGenerator.Create();
        var preRevocationClientActId = _guidGenerator.Create();
        var blockTime = DateTime.UtcNow.AddMinutes(-30);

        // 1. Upload valid block prior to revocation
        var preUploadReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = preRevocationClientActId,
            startedAt = blockTime,
            endedAt = blockTime.AddMinutes(10),
            productivity = 75,
            deviceId = deviceId
        });
        var preResp = await client.SendAsync(preUploadReq);
        preResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Revoke the device
        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new
        {
            deviceId = deviceId,
            reason = "Device decommissioned"
        });
        (await client.SendAsync(revokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. Idempotent replay of previously stored block SUCCEEDS (200 OK)
        var replayReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = preRevocationClientActId,
            startedAt = blockTime,
            endedAt = blockTime.AddMinutes(10),
            productivity = 75,
            deviceId = deviceId
        });
        var replayResp = await client.SendAsync(replayReq);
        replayResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 4. A NEW block from the same revoked device is DENIED (403 Forbidden)
        var newActId = _guidGenerator.Create();
        var newBlockReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = newActId,
            startedAt = blockTime.AddMinutes(10),
            endedAt = blockTime.AddMinutes(20),
            productivity = 75,
            deviceId = deviceId
        });
        var newResp = await client.SendAsync(newBlockReq);
        newResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var newBody = await newResp.Content.ReadAsStringAsync();
        newBody.ShouldContain("Tracker:DeviceRevoked");
    }

    [Fact]
    public async Task Live_Presence_Excludes_Revoked_Device_While_Retaining_Historical_Records()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var revokedDeviceId = _guidGenerator.Create();
        var colleagueDeviceId = _guidGenerator.Create();
        var now = DateTime.UtcNow;

        // 1. Seed presence for UserA (revokedDeviceId) and ColleagueA (colleagueDeviceId)
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pUserA = new AgentPresence(_guidGenerator.Create(), wsA, UserA, revokedDeviceId, "tracking", now)
                {
                    DeviceName = "USER-A-DEV",
                    ProjectId = projA
                };
                var pColleague = new AgentPresence(_guidGenerator.Create(), wsA, ColleagueA, colleagueDeviceId, "tracking", now)
                {
                    DeviceName = "COLLEAGUE-DEV",
                    ProjectId = projA
                };
                await _presenceRepository.InsertAsync(pUserA, autoSave: true);
                await _presenceRepository.InsertAsync(pColleague, autoSave: true);
            }
        });

        // 2. Revoke UserA's device
        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new
        {
            deviceId = revokedDeviceId,
            reason = "Revoking old laptop"
        });
        (await client.SendAsync(revokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. Query live team presence as ColleagueA
        var teamReq = CreateRequest(HttpMethod.Get, "/api/app/presence/team", ColleagueA, wsA);
        var teamResp = await client.SendAsync(teamReq);
        teamResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var teamJson = await teamResp.Content.ReadAsStringAsync();
        using (var doc = JsonDocument.Parse(teamJson))
        {
            var items = doc.RootElement.GetProperty("items").EnumerateArray().ToList();
            var returnedDevices = items.Select(i => Guid.Parse(i.GetProperty("deviceId").GetString()!)).ToList();

            // Revoked device MUST be excluded from live presence!
            returnedDevices.ShouldNotContain(revokedDeviceId);
            // Colleague device MUST be present
            returnedDevices.ShouldContain(colleagueDeviceId);
        }

        // 4. Verify historical presence row still exists in DB (not destroyed)
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var presenceInDb = await _presenceRepository.FirstOrDefaultAsync(p => p.WorkspaceId == wsA && p.UserId == UserA && p.DeviceId == revokedDeviceId);
                presenceInDb.ShouldNotBeNull();
            }
        });
    }

    [Fact]
    public async Task Different_Device_In_A_Works_And_Same_Device_In_B_Is_Unaffected()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var revokedDeviceId = _guidGenerator.Create();
        var secondDeviceId = _guidGenerator.Create();

        // 1. Establish observed revokedDeviceId in wsA and wsB
        var hbA1 = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = revokedDeviceId, status = "tracking" });
        (await client.SendAsync(hbA1)).StatusCode.ShouldBe(HttpStatusCode.OK);

        var hbB1 = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsB, new { deviceId = revokedDeviceId, status = "tracking" });
        (await client.SendAsync(hbB1)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Revoke revokedDeviceId strictly in Workspace A
        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = revokedDeviceId, reason = "Scoped revoke" });
        (await client.SendAsync(revokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. UserA's DIFFERENT device (secondDeviceId) in Workspace A works normally!
        var hbA2 = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = secondDeviceId, status = "tracking" });
        (await client.SendAsync(hbA2)).StatusCode.ShouldBe(HttpStatusCode.OK);

        var actA2 = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = _guidGenerator.Create(),
            startedAt = DateTime.UtcNow.AddMinutes(-5),
            endedAt = DateTime.UtcNow,
            deviceId = secondDeviceId
        });
        (await client.SendAsync(actA2)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 4. UserA's SAME device (revokedDeviceId) in Workspace B is completely UNAFFECTED!
        var hbB2 = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsB, new { deviceId = revokedDeviceId, status = "tracking" });
        (await client.SendAsync(hbB2)).StatusCode.ShouldBe(HttpStatusCode.OK);

        var actB2 = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsB, new
        {
            projectId = projB,
            clientActivityId = _guidGenerator.Create(),
            startedAt = DateTime.UtcNow.AddMinutes(-5),
            endedAt = DateTime.UtcNow,
            deviceId = revokedDeviceId
        });
        (await client.SendAsync(actB2)).StatusCode.ShouldBe(HttpStatusCode.OK);
    }

    [Fact]
    public async Task Denies_Revoking_Another_Members_Device_Or_In_Foreign_Workspace_Without_Tombstones()
    {
        var (wsA, wsB, projA, projB) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var colleagueDeviceId = _guidGenerator.Create();
        var userATargetDevice = _guidGenerator.Create();
        var priorRevokedDevice = _guidGenerator.Create();

        // 1. Establish observed devices and nonempty sentinels in target workspace wsA:
        // A. ColleagueA's device
        var hbColleague = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", ColleagueA, wsA, new { deviceId = colleagueDeviceId, status = "tracking" });
        (await client.SendAsync(hbColleague)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // B. UserA's real observed target device in wsA
        var hbUserA = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = userATargetDevice, status = "tracking" });
        (await client.SendAsync(hbUserA)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // C. Pre-existing revocation sentinel in wsA for UserA
        var hbPrior = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = priorRevokedDevice, status = "tracking" });
        (await client.SendAsync(hbPrior)).StatusCode.ShouldBe(HttpStatusCode.OK);
        var preRevokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = priorRevokedDevice, reason = "Pre-existing revocation sentinel" });
        (await client.SendAsync(preRevokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // D. Preseeded blob sentinel in wsA
        var blobSentinelKey = $"workspaces/{wsA}/sentinel.bin";
        var blobSentinelBytes = new byte[] { 0xAA, 0xBB, 0xCC };
        _blobProvider.Blobs[blobSentinelKey] = (byte[])blobSentinelBytes.Clone();

        // 2. UserA attempts to revoke ColleagueA's device in wsA -> 400 Tracker:DeviceNotObserved (UserA never observed it as own)
        var revokeOtherReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = colleagueDeviceId });
        var revokeOtherResp = await client.SendAsync(revokeOtherReq);
        revokeOtherResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var errBody = await revokeOtherResp.Content.ReadAsStringAsync();
        errBody.ShouldContain("Tracker:DeviceNotObserved");

        // Verify no revocation record created for colleagueDeviceId
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var exists = await _revocationRepository.AnyAsync(r => r.DeviceId == colleagueDeviceId);
                exists.ShouldBeFalse();
            }
        });

        // ColleagueA's device continues to function normally
        var hbColleague2 = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", ColleagueA, wsA, new { deviceId = colleagueDeviceId, status = "tracking" });
        (await client.SendAsync(hbColleague2)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. Real-target foreign-workspace negative persistence case:
        // ForeignUser (who belongs ONLY to wsB, has NO membership in wsA) calls revoke targeting
        // the REAL observed own device of UserA (userATargetDevice) in wsA!
        var foreignTargetReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", ForeignUser, wsA, new
        {
            deviceId = userATargetDevice,
            reason = "Malicious foreign revoke attempt"
        });
        var foreignTargetResp = await client.SendAsync(foreignTargetReq);
        foreignTargetResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var foreignErrBody = await foreignTargetResp.Content.ReadAsStringAsync();
        foreignErrBody.ShouldContain("Tracker:NotWorkspaceMember");

        // 4. Explicitly inspect target workspace wsA afterward to assert NO new tombstones and exact state preservation:
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                // Assert no revocation tombstone was created for ForeignUser in wsA
                var foreignRevocationExists = await _revocationRepository.AnyAsync(r => r.WorkspaceId == wsA && r.UserId == ForeignUser);
                foreignRevocationExists.ShouldBeFalse();

                // Assert userATargetDevice was NOT revoked (remains active and intact)
                var targetRevocationExists = await _revocationRepository.AnyAsync(r => r.WorkspaceId == wsA && r.DeviceId == userATargetDevice);
                targetRevocationExists.ShouldBeFalse();

                // Assert exact total revocation count in wsA is still exactly 1 (only priorRevokedDevice)
                var totalRevocationsInWsA = await _revocationRepository.CountAsync(r => r.WorkspaceId == wsA);
                totalRevocationsInWsA.ShouldBe(1);

                var priorRevocation = await _revocationRepository.FirstOrDefaultAsync(r => r.WorkspaceId == wsA && r.DeviceId == priorRevokedDevice);
                priorRevocation.ShouldNotBeNull();
                priorRevocation.Reason.ShouldBe("Pre-existing revocation sentinel");

                // Assert target presence in wsA survived intact
                var presenceCountInWsA = await _presenceRepository.CountAsync(p => p.WorkspaceId == wsA);
                presenceCountInWsA.ShouldBeGreaterThanOrEqualTo(2);
            }
        });

        // Assert target device userATargetDevice continues to function normally in wsA
        var hbUserAAfter = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = userATargetDevice, status = "tracking" });
        (await client.SendAsync(hbUserAAfter)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // Assert blob sentinel in _blobProvider is preserved
        _blobProvider.Blobs.ContainsKey(blobSentinelKey).ShouldBeTrue();
        _blobProvider.Blobs[blobSentinelKey].SequenceEqual(blobSentinelBytes).ShouldBeTrue();
    }

    [Fact]
    public async Task Presence_Pruning_Does_Not_Erase_Revocation_And_Unobserved_Target_Creates_No_Record()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var deviceId = _guidGenerator.Create();

        // 1. Observe device, then revoke
        var hb = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = deviceId, status = "tracking" });
        (await client.SendAsync(hb)).StatusCode.ShouldBe(HttpStatusCode.OK);

        var revokeReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = deviceId });
        (await client.SendAsync(revokeReq)).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Simulate complete presence cleanup (e.g. stale row deletion)
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var presences = await _presenceRepository.GetListAsync(p => p.WorkspaceId == wsA && p.UserId == UserA && p.DeviceId == deviceId);
                foreach (var p in presences)
                {
                    await _presenceRepository.DeleteAsync(p, autoSave: true);
                }
            }
        });

        // Verify presence rows are gone, but DeviceRevocation remains
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var presenceCount = await _presenceRepository.CountAsync(p => p.WorkspaceId == wsA && p.UserId == UserA && p.DeviceId == deviceId);
                presenceCount.ShouldBe(0);

                var revCount = await _revocationRepository.CountAsync(r => r.WorkspaceId == wsA && r.UserId == UserA && r.DeviceId == deviceId);
                revCount.ShouldBe(1);
            }
        });

        // 3. Post-pruning heartbeat is STILL denied (403 Forbidden)
        var postPruneHb = CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = deviceId, status = "tracking" });
        (await client.SendAsync(postPruneHb)).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 4. Calling revoke with empty or unobserved GUID creates NO record
        var emptyReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = Guid.Empty });
        (await client.SendAsync(emptyReq)).StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        var unobservedId = _guidGenerator.Create();
        var unobsReq = CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = unobservedId });
        (await client.SendAsync(unobsReq)).StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var count = await _revocationRepository.CountAsync(r => r.DeviceId == unobservedId || r.DeviceId == Guid.Empty);
                count.ShouldBe(0);
            }
        });
    }

    [Fact]
    public async Task Status_Inspection_And_Bounded_Listing_Contracts()
    {
        var (wsA, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();
        var deviceId = _guidGenerator.Create();
        var activeDeviceId = _guidGenerator.Create();

        // 1. Observe both devices
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = deviceId, status = "tracking" }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/presence/heartbeat", UserA, wsA, new { deviceId = activeDeviceId, status = "tracking" }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. Check initial status before revoke
        var statusReq1 = CreateRequest(HttpMethod.Get, $"/api/app/device/{deviceId}/status", UserA, wsA);
        var statusResp1 = await client.SendAsync(statusReq1);
        statusResp1.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var doc = JsonDocument.Parse(await statusResp1.Content.ReadAsStringAsync()))
        {
            doc.RootElement.GetProperty("deviceId").GetString().ShouldBe(deviceId.ToString());
            doc.RootElement.GetProperty("isRevoked").GetBoolean().ShouldBeFalse();
        }

        // 3. Revoke deviceId
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke", UserA, wsA, new { deviceId = deviceId, reason = "Decommissioned" }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        // 4. Inspect status after revoke
        var statusReq2 = CreateRequest(HttpMethod.Get, $"/api/app/device/{deviceId}/status", UserA, wsA);
        var statusResp2 = await client.SendAsync(statusReq2);
        statusResp2.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var doc = JsonDocument.Parse(await statusResp2.Content.ReadAsStringAsync()))
        {
            doc.RootElement.GetProperty("deviceId").GetString().ShouldBe(deviceId.ToString());
            doc.RootElement.GetProperty("isRevoked").GetBoolean().ShouldBeTrue();
            doc.RootElement.GetProperty("reason").GetString().ShouldBe("Decommissioned");
            doc.RootElement.TryGetProperty("revokedAt", out var revAt).ShouldBeTrue();
            revAt.GetString().ShouldNotBeNull();
        }

        // 5. Get listing of my revocations
        var listReq = CreateRequest(HttpMethod.Get, "/api/app/device/my-revocations", UserA, wsA);
        var listResp = await client.SendAsync(listReq);
        listResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var doc = JsonDocument.Parse(await listResp.Content.ReadAsStringAsync()))
        {
            var items = doc.RootElement.GetProperty("items").EnumerateArray().ToList();
            items.Count.ShouldBe(1);
            items[0].GetProperty("deviceId").GetString().ShouldBe(deviceId.ToString());
            items[0].GetProperty("reason").GetString().ShouldBe("Decommissioned");
        }
    }

    [Fact]
    public async Task Legacy_Missing_DeviceId_Activity_Behavior_Is_Preserved()
    {
        var (wsA, _, projA, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Activity without deviceId (legacy client) succeeds
        var legacyActReq = CreateRequest(HttpMethod.Post, "/api/app/activity", UserA, wsA, new
        {
            projectId = projA,
            clientActivityId = _guidGenerator.Create(),
            startedAt = DateTime.UtcNow.AddMinutes(-10),
            endedAt = DateTime.UtcNow,
            productivity = 50
        });
        var actResp = await client.SendAsync(legacyActReq);
        actResp.StatusCode.ShouldBe(HttpStatusCode.OK);
    }
}
