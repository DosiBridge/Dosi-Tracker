using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using OpenIddict.Abstractions;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.OpenIddict.Authorizations;
using Volo.Abp.OpenIddict.Tokens;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>
/// Device enrollment, binding and revocation over HTTP (BE-04 / E2E-09 / API-02):
/// 1. A revoked install's device-bound token is refused on every call in that workspace (403 Tracker:DeviceRevoked),
///    while the member's other installs and other workspaces keep working.
/// 2. Naming a fresh device id in the body does not get a revoked token's data in, and the body's revoked id is
///    refused even under an unrevoked token.
/// 3. Owners and managers revoke and restore members' devices (Tracker.Devices.Manage); members cannot, managers cannot
///    touch an owner's device, and a device never seen in the workspace is refused without a record.
/// 4. Revoking a device revokes the OpenIddict authorizations and tokens bound to it, and only those.
/// 5. The device list shows a member their own devices and a manager everyone's, revoked ones included.
/// </summary>
public class WorkspaceDeviceManagementHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Manager = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Member = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid OtherOwner = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb04");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceDeviceManagementHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _revocationRepository = GetRequiredService<IRepository<DeviceRevocation, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var connection = ServiceProvider.GetRequiredService<TrackerDbContext>().Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
            options.Configure(ctx => ctx.DbContextOptions.UseSqlite(connection)));
        services.AddAlwaysDisableUnitOfWorkTransaction();
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB)> SeedAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            foreach (var (id, name) in new[] { (Owner, "dev_owner"), (Manager, "dev_manager"), (Member, "dev_member"), (OtherOwner, "dev_other") })
            {
                if (await _userManager.FindByIdAsync(id.ToString()) == null)
                {
                    await _userManager.CreateAsync(new IdentityUser(id, name, name + "@test.com") { Name = name.ToUpperInvariant() });
                }
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Devices Alpha " + wsA.ToString("N")[..6]), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Devices Beta " + wsB.ToString("N")[..6]), autoSave: true);

            var now = DateTime.UtcNow;
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, Owner, WorkspaceRoles.Owner, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, Manager, WorkspaceRoles.Manager, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, Member, WorkspaceRoles.Member, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, OtherOwner, WorkspaceRoles.Owner, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, Member, WorkspaceRoles.Member, now), autoSave: true);
        });

        return (wsA, wsB);
    }

    /// <summary>A request whose synthetic token is bound to <paramref name="tokenDevice"/> (a desktop agent's).</summary>
    private static HttpRequestMessage AgentRequest(HttpMethod method, string url, Guid userId, Guid workspaceId, Guid? tokenDevice, object? body = null)
    {
        var request = CreateRequest(method, url, userId, workspaceId, body);
        if (tokenDevice.HasValue)
        {
            request.Headers.Add(TestAuthHandler.DeviceHeaderName, tokenDevice.Value.ToString());
        }

        return request;
    }

    private static async Task HeartbeatAsync(HttpClient client, Guid userId, Guid workspaceId, Guid deviceId, string? name = null)
    {
        var response = await client.SendAsync(AgentRequest(HttpMethod.Post, "/api/app/presence/heartbeat", userId, workspaceId, deviceId,
            new { deviceId, status = "tracking", deviceName = name }));
        response.StatusCode.ShouldBe(HttpStatusCode.OK, await response.Content.ReadAsStringAsync());
    }

    private static async Task<string?> ErrorCodeAsync(HttpResponseMessage response)
    {
        var body = await response.Content.ReadAsStringAsync();
        using var doc = JsonDocument.Parse(body);
        return doc.RootElement.TryGetProperty("error", out var error) && error.TryGetProperty("code", out var code)
            ? code.GetString()
            : null;
    }

    [Fact]
    public async Task A_Revoked_Device_Bound_Token_Is_Refused_On_Every_Call_In_That_Workspace_Only()
    {
        var (wsA, wsB) = await SeedAsync();
        var client = CreateHttpClient();
        var laptop = _guidGenerator.Create();
        var desktop = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsA, laptop);
        await HeartbeatAsync(client, Member, wsA, desktop);
        await HeartbeatAsync(client, Member, wsB, laptop);

        var revoke = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Owner, wsA,
            new { userId = Member, deviceId = laptop, reason = "Lost on a train" }));
        revoke.StatusCode.ShouldBe(HttpStatusCode.OK, await revoke.Content.ReadAsStringAsync());
        using (var doc = JsonDocument.Parse(await revoke.Content.ReadAsStringAsync()))
        {
            doc.RootElement.GetProperty("userId").GetGuid().ShouldBe(Member);
            doc.RootElement.GetProperty("revokedByUserId").GetGuid().ShouldBe(Owner);
        }

        // Any call in A with the laptop's token: reads included, not only uploads.
        foreach (var (method, url) in new[] { (HttpMethod.Get, "/api/app/project/my-projects"), (HttpMethod.Get, "/api/app/device"), (HttpMethod.Get, $"/api/app/device/{laptop}/status") })
        {
            var refused = await client.SendAsync(AgentRequest(method, url, Member, wsA, laptop));
            refused.StatusCode.ShouldBe(HttpStatusCode.Forbidden, url);
            (await ErrorCodeAsync(refused)).ShouldBe(TrackerDomainErrorCodes.DeviceRevoked);
        }

        // The member's other install, the same install in another workspace, and the member's web session still work.
        (await client.SendAsync(AgentRequest(HttpMethod.Get, "/api/app/project/my-projects", Member, wsA, desktop))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(AgentRequest(HttpMethod.Get, "/api/app/project/my-projects", Member, wsB, laptop))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/project/my-projects", Member, wsA))).StatusCode.ShouldBe(HttpStatusCode.OK);
    }

    [Fact]
    public async Task A_Fresh_Or_Omitted_Body_Device_Id_Does_Not_Bypass_A_Revocation()
    {
        var (wsA, _) = await SeedAsync();
        var client = CreateHttpClient();
        var revoked = _guidGenerator.Create();
        var other = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsA, revoked);
        (await client.SendAsync(AgentRequest(HttpMethod.Post, "/api/app/device/revoke", Member, wsA, other, new { deviceId = revoked })))
            .StatusCode.ShouldBe(HttpStatusCode.OK);

        // Revoked token, invented body id: refused before the upload is looked at.
        var invented = await client.SendAsync(AgentRequest(HttpMethod.Post, "/api/app/presence/heartbeat", Member, wsA, revoked,
            new { deviceId = _guidGenerator.Create(), status = "tracking" }));
        (await ErrorCodeAsync(invented)).ShouldBe(TrackerDomainErrorCodes.DeviceRevoked);

        // Unrevoked token, revoked body id (a queued row of the revoked install): refused as well.
        var queued = await client.SendAsync(AgentRequest(HttpMethod.Post, "/api/app/presence/heartbeat", Member, wsA, other,
            new { deviceId = revoked, status = "tracking" }));
        queued.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await ErrorCodeAsync(queued)).ShouldBe(TrackerDomainErrorCodes.DeviceRevoked);

        var upload = await client.SendAsync(AgentRequest(HttpMethod.Post, "/api/app/activity", Member, wsA, other, new
        {
            projectId = _guidGenerator.Create(),
            clientActivityId = _guidGenerator.Create(),
            startedAt = DateTime.UtcNow.AddMinutes(-20),
            endedAt = DateTime.UtcNow.AddMinutes(-10),
            deviceId = revoked
        }));
        upload.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await ErrorCodeAsync(upload)).ShouldBe(TrackerDomainErrorCodes.DeviceRevoked);
    }

    [Fact]
    public async Task Only_Owners_And_Managers_Manage_Members_Devices_And_Managers_Never_An_Owners()
    {
        var (wsA, _) = await SeedAsync();
        var client = CreateHttpClient();
        var ownerDevice = _guidGenerator.Create();
        var managerDevice = _guidGenerator.Create();
        var memberDevice = _guidGenerator.Create();
        await HeartbeatAsync(client, Owner, wsA, ownerDevice);
        await HeartbeatAsync(client, Manager, wsA, managerDevice);
        await HeartbeatAsync(client, Member, wsA, memberDevice);

        // A member has no Tracker.Devices.Manage: neither revoke nor restore of someone else's device.
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Member, wsA,
            new { userId = Manager, deviceId = managerDevice }))).StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/restore", Member, wsA,
            new { userId = Manager, deviceId = managerDevice }))).StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // A manager may not revoke an owner's device; an owner may revoke a manager's.
        var managerOnOwner = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Manager, wsA,
            new { userId = Owner, deviceId = ownerDevice }));
        managerOnOwner.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await ErrorCodeAsync(managerOnOwner)).ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);

        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Owner, wsA,
            new { userId = Manager, deviceId = managerDevice }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Manager, wsA,
            new { userId = Member, deviceId = memberDevice }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                (await _revocationRepository.GetListAsync()).Select(r => r.DeviceId)
                    .ShouldBe(new[] { managerDevice, memberDevice }, ignoreOrder: true);
            }
        });
    }

    [Fact]
    public async Task Restore_Lifts_A_Revocation_And_Answers_404_When_There_Is_None()
    {
        var (wsA, _) = await SeedAsync();
        var client = CreateHttpClient();
        var device = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsA, device);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Manager, wsA,
            new { userId = Member, deviceId = device }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(AgentRequest(HttpMethod.Get, "/api/app/project/my-projects", Member, wsA, device)))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        var restored = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/restore", Manager, wsA,
            new { userId = Member, deviceId = device }));
        restored.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var doc = JsonDocument.Parse(await restored.Content.ReadAsStringAsync()))
        {
            doc.RootElement.GetProperty("isRevoked").GetBoolean().ShouldBeFalse();
        }

        (await client.SendAsync(AgentRequest(HttpMethod.Get, "/api/app/project/my-projects", Member, wsA, device)))
            .StatusCode.ShouldBe(HttpStatusCode.OK);
        await HeartbeatAsync(client, Member, wsA, device);

        var again = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/restore", Manager, wsA,
            new { userId = Member, deviceId = device }));
        again.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await ErrorCodeAsync(again)).ShouldBe(TrackerDomainErrorCodes.DeviceNotFound);
    }

    [Fact]
    public async Task A_Device_Never_Seen_In_The_Workspace_Cannot_Be_Revoked_There()
    {
        var (wsA, wsB) = await SeedAsync();
        var client = CreateHttpClient();
        var onlyInB = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsB, onlyInB);

        // The owner of A cannot sign the member's B-only install out through A.
        var response = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Owner, wsA,
            new { userId = Member, deviceId = onlyInB }));
        response.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        (await ErrorCodeAsync(response)).ShouldBe(TrackerDomainErrorCodes.DeviceNotObserved);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                (await _revocationRepository.GetCountAsync()).ShouldBe(0);
            }
        });
    }

    [Fact]
    public async Task Revoking_A_Device_Revokes_The_Sessions_Bound_To_It_And_Only_Those()
    {
        var (wsA, _) = await SeedAsync();
        var client = CreateHttpClient();
        var laptop = _guidGenerator.Create();
        var desktop = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsA, laptop);

        var laptopSession = await CreateSessionAsync(Member, laptop);
        var desktopSession = await CreateSessionAsync(Member, desktop);
        var webSession = await CreateSessionAsync(Member, null);
        var ownersLaptopId = await CreateSessionAsync(Owner, laptop);

        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Owner, wsA,
            new { userId = Member, deviceId = laptop }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        (await SessionStatusesAsync(laptopSession)).ShouldBe((OpenIddictConstants.Statuses.Revoked, OpenIddictConstants.Statuses.Revoked));
        (await SessionStatusesAsync(desktopSession)).ShouldBe((OpenIddictConstants.Statuses.Valid, OpenIddictConstants.Statuses.Valid));
        (await SessionStatusesAsync(webSession)).ShouldBe((OpenIddictConstants.Statuses.Valid, OpenIddictConstants.Statuses.Valid));
        (await SessionStatusesAsync(ownersLaptopId)).ShouldBe((OpenIddictConstants.Statuses.Valid, OpenIddictConstants.Statuses.Valid));

        // The member signs the laptop in again with their password; revoking it again ends that session too.
        var newSignIn = await CreateSessionAsync(Member, laptop);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Owner, wsA,
            new { userId = Member, deviceId = laptop }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await SessionStatusesAsync(newSignIn)).ShouldBe((OpenIddictConstants.Statuses.Revoked, OpenIddictConstants.Statuses.Revoked));
    }

    [Fact]
    public async Task The_Device_List_Shows_Members_Their_Own_And_Managers_Everyones()
    {
        var (wsA, wsB) = await SeedAsync();
        var client = CreateHttpClient();
        var memberLaptop = _guidGenerator.Create();
        var memberOld = _guidGenerator.Create();
        var managerDevice = _guidGenerator.Create();
        await HeartbeatAsync(client, Member, wsA, memberLaptop, "MEMBER-LAPTOP");
        await HeartbeatAsync(client, Member, wsA, memberOld, "MEMBER-OLD");
        await HeartbeatAsync(client, Manager, wsA, managerDevice, "MANAGER-PC");
        await HeartbeatAsync(client, Member, wsB, _guidGenerator.Create(), "MEMBER-B");
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/device/revoke-member", Manager, wsA,
            new { userId = Member, deviceId = memberOld, reason = "Replaced" }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        var mine = await ListAsync(client, Member, wsA);
        mine.Select(d => d.GetProperty("deviceId").GetGuid()).ShouldBe(new[] { memberLaptop, memberOld });
        mine[0].GetProperty("deviceName").GetString().ShouldBe("MEMBER-LAPTOP");
        mine[0].GetProperty("isRevoked").GetBoolean().ShouldBeFalse();
        mine[0].GetProperty("userName").GetString().ShouldBe("dev_member");
        mine[1].GetProperty("isRevoked").GetBoolean().ShouldBeTrue();
        mine[1].GetProperty("reason").GetString().ShouldBe("Replaced");
        mine[1].GetProperty("revokedByUserId").GetGuid().ShouldBe(Manager);

        var all = await ListAsync(client, Manager, wsA);
        all.Select(d => d.GetProperty("deviceId").GetGuid()).ShouldBe(new[] { memberLaptop, managerDevice, memberOld }, ignoreOrder: true);
        all.Last().GetProperty("deviceId").GetGuid().ShouldBe(memberOld); // revoked devices come last

        // A revoked device stays listed after presence forgot it.
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA))
            {
                var presence = GetRequiredService<IRepository<Dosi.Tracker.Presence.AgentPresence, Guid>>();
                await presence.DeleteAsync(p => p.DeviceId == memberOld);
            }
        });
        var afterPrune = await ListAsync(client, Member, wsA);
        var forgotten = afterPrune.Single(d => d.GetProperty("deviceId").GetGuid() == memberOld);
        forgotten.GetProperty("isRevoked").GetBoolean().ShouldBeTrue();
        forgotten.GetProperty("lastSeenAt").ValueKind.ShouldBe(JsonValueKind.Null);
    }

    private static async Task<JsonElement[]> ListAsync(HttpClient client, Guid userId, Guid workspaceId)
    {
        var response = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/device", userId, workspaceId));
        var body = await response.Content.ReadAsStringAsync();
        response.StatusCode.ShouldBe(HttpStatusCode.OK, body);
        return JsonDocument.Parse(body).RootElement.GetProperty("items").EnumerateArray().Select(e => e.Clone()).ToArray();
    }

    /// <summary>A sign-in session as OpenIddict stores it: an authorization (bound to <paramref name="deviceId"/> when
    /// given) with a refresh token.</summary>
    private async Task<(string AuthorizationId, string TokenId)> CreateSessionAsync(Guid userId, Guid? deviceId)
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            var authorizations = GetRequiredService<IOpenIddictAuthorizationManager>();
            var tokens = GetRequiredService<IOpenIddictTokenManager>();

            var descriptor = new OpenIddictAuthorizationDescriptor
            {
                CreationDate = DateTimeOffset.UtcNow,
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = userId.ToString(),
                Type = OpenIddictConstants.AuthorizationTypes.AdHoc
            };
            if (deviceId.HasValue)
            {
                descriptor.Properties[DeviceConsts.AuthorizationProperty] =
                    JsonSerializer.SerializeToElement(DeviceTokenBinding.Format(deviceId.Value));
            }

            var authorization = await authorizations.CreateAsync(descriptor);
            var authorizationId = (await authorizations.GetIdAsync(authorization))!;
            var token = await tokens.CreateAsync(new OpenIddictTokenDescriptor
            {
                AuthorizationId = authorizationId,
                CreationDate = DateTimeOffset.UtcNow,
                ExpirationDate = DateTimeOffset.UtcNow.AddDays(14),
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = userId.ToString(),
                Type = OpenIddictConstants.TokenTypeHints.RefreshToken
            });

            return (authorizationId, (await tokens.GetIdAsync(token))!);
        });
    }

    /// <summary>The stored statuses, read from the database: OpenIddict's entity cache of this test container never saw
    /// the server's revocation.</summary>
    private async Task<(string? Authorization, string? Token)> SessionStatusesAsync((string AuthorizationId, string TokenId) session)
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            var authorization = await GetRequiredService<IOpenIddictAuthorizationRepository>().GetAsync(Guid.Parse(session.AuthorizationId));
            var token = await GetRequiredService<IOpenIddictTokenRepository>().GetAsync(Guid.Parse(session.TokenId));
            return (authorization.Status, token.Status);
        });
    }
}
