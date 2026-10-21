using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.EntityFrameworkCore;
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
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// REQ-ISO-05 / Private-Capture-Delivery acceptance harness:
/// Concrete hosted HTTP isolation regressions for the authenticated binary file endpoint:
///   GET /api/app/activity/screenshot/{id}/content
/// 
/// Tests genuine hosted TestServer HTTP pipeline, production WorkspaceResolutionMiddleware,
/// WorkspaceRequiredFilter, aggregate workspace permissions (WorkspaceRolePermissionValueProvider),
/// real ActivityAppService / EF Core global query filters, and instrumented in-memory ScriptedBlobProvider.
/// </summary>
public class WorkspaceScreenshotContentHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid DualUser = Guid.Parse("7b801e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid UserAOnly = Guid.Parse("7b801e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid UserBOther = Guid.Parse("7b801e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid InactiveUserA = Guid.Parse("7b801e62-0953-4dd3-910b-dc6cc93ccb04");

    private readonly ScriptedBlobProvider _blobProvider = new();

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceScreenshotContentHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _projectMemberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
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
        var req = WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("*/*"));
        return req;
    }

    private static HttpRequestMessage CreateCustomRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        string? rawWorkspaceHeader)
    {
        var request = new HttpRequestMessage(method, url);
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("*/*"));
        if (userId.HasValue)
        {
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", userId.Value.ToString());
        }

        if (rawWorkspaceHeader != null)
        {
            request.Headers.Add(WorkspaceConsts.HeaderName, rawWorkspaceHeader);
        }

        return request;
    }

    /// <summary>
    /// Fails closed if CacheControl header is absent, missing no-store, or missing private.
    /// Used across all capture delivery responses (both allowed 200 binary streams and 401/400/403/404 rejection paths).
    /// </summary>
    private static void AssertNonStorablePrivateCachePolicy(HttpResponseMessage response)
    {
        response.Headers.CacheControl.ShouldNotBeNull();
        response.Headers.CacheControl.NoStore.ShouldBeTrue();
        response.Headers.CacheControl.Private.ShouldBeTrue();
    }

    private sealed record FixtureData(
        Guid WsA,
        Guid WsB,
        Guid ProjA,
        Guid ProjB,
        Guid ActA_Dual,
        Guid ScA_Dual,
        byte[] BytesA_Dual,
        string StorageUrlA_Dual,
        Guid ActA_Only,
        Guid ScA_Only,
        byte[] BytesA_Only,
        string StorageUrlA_Only,
        Guid ActB_Dual,
        Guid ScB_Dual,
        byte[] BytesB_Dual,
        string StorageUrlB_Dual,
        Guid ActB_Other,
        Guid ScB_Other,
        byte[] BytesB_Other,
        string StorageUrlB_Other,
        Guid ActB_MissingBlob,
        Guid ScB_MissingBlob,
        string StorageUrlB_MissingBlob
    );

    private async Task<FixtureData> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();
        var projA = _guidGenerator.Create();
        var projB = _guidGenerator.Create();

        var actA_Dual = _guidGenerator.Create();
        var scA_Dual = _guidGenerator.Create();
        var storageUrlA_Dual = $"activities/{actA_Dual}/screen.jpg";
        var bytesA_Dual = new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x01, 0x00, 0x60, 0x00, 0x60, 0x00, 0x00, 0xAA, 0x11, 0x22 };

        var actA_Only = _guidGenerator.Create();
        var scA_Only = _guidGenerator.Create();
        var storageUrlA_Only = $"activities/{actA_Only}/screen.jpg";
        var bytesA_Only = new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x01, 0x00, 0x60, 0x00, 0x60, 0x00, 0x00, 0xBB, 0x33, 0x44 };

        var actB_Dual = _guidGenerator.Create();
        var scB_Dual = _guidGenerator.Create();
        var storageUrlB_Dual = $"activities/{actB_Dual}/screen.png";
        var bytesB_Dual = new byte[] { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0xCC, 0x55, 0x66 };

        var actB_Other = _guidGenerator.Create();
        var scB_Other = _guidGenerator.Create();
        var storageUrlB_Other = $"activities/{actB_Other}/screen.png";
        var bytesB_Other = new byte[] { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0xDD, 0x77, 0x88 };

        var actB_MissingBlob = _guidGenerator.Create();
        var scB_MissingBlob = _guidGenerator.Create();
        var storageUrlB_MissingBlob = $"activities/{actB_MissingBlob}/screen.jpg";

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(DualUser, "dual_user", "dual_user@test.com"));
            }
            if (await _userManager.FindByIdAsync(UserAOnly.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserAOnly, "usera_only", "usera_only@test.com"));
            }
            if (await _userManager.FindByIdAsync(UserBOther.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(UserBOther, "userb_other", "userb_other@test.com"));
            }
            if (await _userManager.FindByIdAsync(InactiveUserA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(InactiveUserA, "inactive_usera", "inactive_usera@test.com"));
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace Alpha"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace Beta"), autoSave: true);

            var now = DateTime.UtcNow;

            // Workspace A members: DualUser (Owner), UserAOnly (Member), InactiveUserA (Inactive Member)
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Owner, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, UserAOnly, WorkspaceRoles.Member, now), autoSave: true);
            var inactiveMember = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveUserA, WorkspaceRoles.Member, now);
            inactiveMember.Remove();
            await _memberRepository.InsertAsync(inactiveMember, autoSave: true);

            // Workspace B members: DualUser (Member), UserBOther (Owner)
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, now), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, UserBOther, WorkspaceRoles.Owner, now), autoSave: true);

            // Seed Workspace A Projects, Activities, Screenshots
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                await _projectRepository.InsertAsync(new Project(projA, wsA, "Project Alpha"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA, DualUser, "Lead"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA, UserAOnly, "Developer"), autoSave: true);

                var actDualA = new Activity(actA_Dual, wsA, DualUser, projA, _guidGenerator.Create(), now.AddMinutes(-30), now.AddMinutes(-20));
                await _activityRepository.InsertAsync(actDualA, autoSave: true);
                var scDualA = new Screenshot(scA_Dual, wsA, actA_Dual, storageUrlA_Dual, now.AddMinutes(-25), bytesA_Dual.Length, "image/jpeg", Screenshot.ScreenKind);
                await _screenshotRepository.InsertAsync(scDualA, autoSave: true);

                var actOnlyA = new Activity(actA_Only, wsA, UserAOnly, projA, _guidGenerator.Create(), now.AddMinutes(-20), now.AddMinutes(-10));
                await _activityRepository.InsertAsync(actOnlyA, autoSave: true);
                var scOnlyA = new Screenshot(scA_Only, wsA, actA_Only, storageUrlA_Only, now.AddMinutes(-15), bytesA_Only.Length, "image/jpeg", Screenshot.ScreenKind);
                await _screenshotRepository.InsertAsync(scOnlyA, autoSave: true);
            }

            // Seed Workspace B Projects, Activities, Screenshots
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                await _projectRepository.InsertAsync(new Project(projB, wsB, "Project Beta"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB, DualUser, "Developer"), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB, UserBOther, "Lead"), autoSave: true);

                var actDualB = new Activity(actB_Dual, wsB, DualUser, projB, _guidGenerator.Create(), now.AddMinutes(-30), now.AddMinutes(-20));
                await _activityRepository.InsertAsync(actDualB, autoSave: true);
                var scDualB = new Screenshot(scB_Dual, wsB, actB_Dual, storageUrlB_Dual, now.AddMinutes(-25), bytesB_Dual.Length, "image/png", Screenshot.ScreenKind);
                await _screenshotRepository.InsertAsync(scDualB, autoSave: true);

                var actOtherB = new Activity(actB_Other, wsB, UserBOther, projB, _guidGenerator.Create(), now.AddMinutes(-20), now.AddMinutes(-10));
                await _activityRepository.InsertAsync(actOtherB, autoSave: true);
                var scOtherB = new Screenshot(scB_Other, wsB, actB_Other, storageUrlB_Other, now.AddMinutes(-15), bytesB_Other.Length, "image/png", Screenshot.ScreenKind);
                await _screenshotRepository.InsertAsync(scOtherB, autoSave: true);

                var actMissingB = new Activity(actB_MissingBlob, wsB, DualUser, projB, _guidGenerator.Create(), now.AddMinutes(-10), now);
                await _activityRepository.InsertAsync(actMissingB, autoSave: true);
                var scMissingB = new Screenshot(scB_MissingBlob, wsB, actB_MissingBlob, storageUrlB_MissingBlob, now.AddMinutes(-5), 100, "image/jpeg", Screenshot.ScreenKind);
                await _screenshotRepository.InsertAsync(scMissingB, autoSave: true);
            }
        });

        // Preseed blobs in the instrumented provider (actB_MissingBlob is deliberately omitted)
        _blobProvider.Blobs[storageUrlA_Dual] = (byte[])bytesA_Dual.Clone();
        _blobProvider.Blobs[storageUrlA_Only] = (byte[])bytesA_Only.Clone();
        _blobProvider.Blobs[storageUrlB_Dual] = (byte[])bytesB_Dual.Clone();
        _blobProvider.Blobs[storageUrlB_Other] = (byte[])bytesB_Other.Clone();

        return new FixtureData(
            wsA, wsB, projA, projB,
            actA_Dual, scA_Dual, bytesA_Dual, storageUrlA_Dual,
            actA_Only, scA_Only, bytesA_Only, storageUrlA_Only,
            actB_Dual, scB_Dual, bytesB_Dual, storageUrlB_Dual,
            actB_Other, scB_Other, bytesB_Other, storageUrlB_Other,
            actB_MissingBlob, scB_MissingBlob, storageUrlB_MissingBlob
        );
    }

    [Fact]
    public void NonStorablePrivateCachePolicyOracle_RejectsMissingOrCacheableHeaders_AndAcceptsStrictNonStorable()
    {
        // 1. Oracle must reject response with no Cache-Control header (null)
        using var respNoHeader = new HttpResponseMessage(HttpStatusCode.Unauthorized);
        respNoHeader.Headers.CacheControl.ShouldBeNull();
        Should.Throw<ShouldAssertException>(() => AssertNonStorablePrivateCachePolicy(respNoHeader));

        // 2. Oracle must reject response with public/cacheable max-age policy
        using var respPublic = new HttpResponseMessage(HttpStatusCode.OK);
        respPublic.Headers.CacheControl = new CacheControlHeaderValue
        {
            Public = true,
            MaxAge = TimeSpan.FromHours(1)
        };
        Should.Throw<ShouldAssertException>(() => AssertNonStorablePrivateCachePolicy(respPublic));

        // 3. Oracle must reject response missing Private directive
        using var respNoStoreOnly = new HttpResponseMessage(HttpStatusCode.OK);
        respNoStoreOnly.Headers.CacheControl = new CacheControlHeaderValue
        {
            NoStore = true,
            Private = false
        };
        Should.Throw<ShouldAssertException>(() => AssertNonStorablePrivateCachePolicy(respNoStoreOnly));

        // 4. Oracle must reject response missing NoStore directive
        using var respPrivateOnly = new HttpResponseMessage(HttpStatusCode.OK);
        respPrivateOnly.Headers.CacheControl = new CacheControlHeaderValue
        {
            NoStore = false,
            Private = true
        };
        Should.Throw<ShouldAssertException>(() => AssertNonStorablePrivateCachePolicy(respPrivateOnly));

        // 5. Oracle must accept compliant no-store, private response
        using var respCompliant = new HttpResponseMessage(HttpStatusCode.OK);
        respCompliant.Headers.CacheControl = new CacheControlHeaderValue
        {
            NoStore = true,
            Private = true
        };
        AssertNonStorablePrivateCachePolicy(respCompliant);
    }

    [Fact]
    public async Task GetScreenshotContent_Unauthenticated_Returns401Unauthorized()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var req = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", userId: null, workspaceId: fix.WsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        AssertNonStorablePrivateCachePolicy(resp);
        _blobProvider.GetCount.ShouldBe(0);
        _blobProvider.GetAttempts.ShouldBeEmpty();
    }

    [Fact]
    public async Task GetScreenshotContent_MissingOrInvalidWorkspaceHeader_Returns400BadRequest()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Missing workspace header
        var reqMissing = CreateCustomRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, rawWorkspaceHeader: null);
        var respMissing = await client.SendAsync(reqMissing);
        respMissing.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        AssertNonStorablePrivateCachePolicy(respMissing);
        var bodyMissing = await respMissing.Content.ReadAsStringAsync();
        bodyMissing.ShouldContain("Tracker:WorkspaceRequired");

        // 2. Whitespace-only workspace header
        var reqEmpty = CreateCustomRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, rawWorkspaceHeader: "   ");
        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        AssertNonStorablePrivateCachePolicy(respEmpty);
        var bodyEmpty = await respEmpty.Content.ReadAsStringAsync();
        bodyEmpty.ShouldContain("Tracker:WorkspaceRequired");

        // 3. Malformed GUID workspace header
        var reqMalformed = CreateCustomRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, rawWorkspaceHeader: "not-a-guid");
        var respMalformed = await client.SendAsync(reqMalformed);
        respMalformed.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        AssertNonStorablePrivateCachePolicy(respMalformed);
        var bodyMalformed = await respMalformed.Content.ReadAsStringAsync();
        bodyMalformed.ShouldContain("Tracker:WorkspaceRequired");

        // 4. Empty GUID (Guid.Empty) workspace header
        var reqZero = CreateCustomRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, rawWorkspaceHeader: Guid.Empty.ToString());
        var respZero = await client.SendAsync(reqZero);
        respZero.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        AssertNonStorablePrivateCachePolicy(respZero);
        var bodyZero = await respZero.Content.ReadAsStringAsync();
        bodyZero.ShouldContain("Tracker:WorkspaceRequired");

        _blobProvider.GetCount.ShouldBe(0);
        _blobProvider.GetAttempts.ShouldBeEmpty();
    }

    [Fact]
    public async Task GetScreenshotContent_AOnlyCaller_SpoofingWorkspaceBHeader_Returns403Forbidden()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // UserAOnly is only in wsA, attempts to request under wsB
        var req = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", UserAOnly, fix.WsB);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        AssertNonStorablePrivateCachePolicy(resp);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("Tracker:NotWorkspaceMember");

        _blobProvider.GetCount.ShouldBe(0);
        _blobProvider.GetAttempts.ShouldBeEmpty();
    }

    [Fact]
    public async Task GetScreenshotContent_CrossWorkspaceAccess_UnderWorkspaceA_Returns404NotFound()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // DualUser (or UserAOnly) sends request under Workspace A targeting a Workspace B screenshot ID
        var req = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", DualUser, fix.WsA);
        var resp = await client.SendAsync(req);

        // EF Core global query filter prevents discovering B's screenshot under A -> non-disclosing 404
        resp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(resp);

        // Body must NOT leak binary image data, storage URL, or foreign activity identifiers
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldNotContain(fix.StorageUrlB_Dual);
        body.ShouldNotContain(fix.ActB_Dual.ToString());

        // Provider must NOT have been called for Workspace B's blob
        _blobProvider.GetAttempts.Contains(fix.StorageUrlB_Dual).ShouldBeFalse();
        _blobProvider.GetCount.ShouldBe(0);
    }

    [Fact]
    public async Task GetScreenshotContent_RegularMemberInWorkspaceB_CannotReadOtherMembersCapture_WhileOwnRemainsAccessible()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // In Workspace B, DualUser is a regular 'Member' (no TrackerPermissions.Activities.ViewAll)
        // 1. DualUser attempts to read UserBOther's capture in wsB -> 404 Not Found (EnsureCanReadActivityAsync hides it)
        var reqOther = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Other}/content", DualUser, fix.WsB);
        var respOther = await client.SendAsync(reqOther);

        respOther.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(respOther);
        var bodyOther = await respOther.Content.ReadAsStringAsync();
        bodyOther.ShouldNotContain(fix.StorageUrlB_Other);
        _blobProvider.GetAttempts.Contains(fix.StorageUrlB_Other).ShouldBeFalse();
        _blobProvider.GetCount.ShouldBe(0);

        // 2. Control: DualUser reads OWN capture in wsB -> 200 OK with correct binary bytes and detected PNG content type
        var reqOwn = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", DualUser, fix.WsB);
        var respOwn = await client.SendAsync(reqOwn);

        respOwn.StatusCode.ShouldBe(HttpStatusCode.OK);
        AssertNonStorablePrivateCachePolicy(respOwn);
        respOwn.Content.Headers.ContentType?.MediaType.ShouldBe("image/png");

        var receivedBytes = await respOwn.Content.ReadAsByteArrayAsync();
        receivedBytes.SequenceEqual(fix.BytesB_Dual).ShouldBeTrue();

        // Instrumentation proof: GetCount incremented exactly once for the authorized own blob
        _blobProvider.GetCount.ShouldBe(1);
        _blobProvider.GetAttempts.ToList().ShouldBe(new[] { fix.StorageUrlB_Dual });
    }

    [Fact]
    public async Task GetScreenshotContent_LegitimateDualWorkspaceSwitching_RetrievesAuthorizedBytesAndContentType()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. DualUser operates in Workspace A (as Owner, holding TrackerPermissions.Activities.ViewAll):
        // A. Read own capture in wsA
        var reqOwnA = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, fix.WsA);
        var respOwnA = await client.SendAsync(reqOwnA);
        respOwnA.StatusCode.ShouldBe(HttpStatusCode.OK);
        respOwnA.Content.Headers.ContentType?.MediaType.ShouldBe("image/jpeg");
        var bytesOwnA = await respOwnA.Content.ReadAsByteArrayAsync();
        bytesOwnA.SequenceEqual(fix.BytesA_Dual).ShouldBeTrue();
        respOwnA.Content.Headers.ContentDisposition?.FileName.ShouldBe($"{fix.ActA_Dual}-screen.jpg");
        // Inspect response cache policy: private capture delivery must enforce explicit non-storable caching
        AssertNonStorablePrivateCachePolicy(respOwnA);

        // B. Read colleague's capture in wsA (allowed because Owner has ViewAll in wsA)
        var reqOnlyA = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Only}/content", DualUser, fix.WsA);
        var respOnlyA = await client.SendAsync(reqOnlyA);
        respOnlyA.StatusCode.ShouldBe(HttpStatusCode.OK);
        AssertNonStorablePrivateCachePolicy(respOnlyA);
        respOnlyA.Content.Headers.ContentType?.MediaType.ShouldBe("image/jpeg");
        var bytesOnlyA = await respOnlyA.Content.ReadAsByteArrayAsync();
        bytesOnlyA.SequenceEqual(fix.BytesA_Only).ShouldBeTrue();
        respOnlyA.Content.Headers.ContentDisposition?.FileName.ShouldBe($"{fix.ActA_Only}-screen.jpg");

        // 2. DualUser switches context to Workspace B (as regular Member):
        // Read own capture in wsB -> binary PNG bytes
        var reqOwnB = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", DualUser, fix.WsB);
        var respOwnB = await client.SendAsync(reqOwnB);
        respOwnB.StatusCode.ShouldBe(HttpStatusCode.OK);
        AssertNonStorablePrivateCachePolicy(respOwnB);
        respOwnB.Content.Headers.ContentType?.MediaType.ShouldBe("image/png");
        var bytesOwnB = await respOwnB.Content.ReadAsByteArrayAsync();
        bytesOwnB.SequenceEqual(fix.BytesB_Dual).ShouldBeTrue();
        respOwnB.Content.Headers.ContentDisposition?.FileName.ShouldBe($"{fix.ActB_Dual}-screen.png");

        // Verify distinct binary payloads across workspaces for the same user
        bytesOwnA.SequenceEqual(bytesOwnB).ShouldBeFalse("Workspace A JPEG and Workspace B PNG must be distinct payloads");

        // Verify provider instrumentation observed all 3 legitimate gets in order
        _blobProvider.GetCount.ShouldBe(3);
        var attempts = _blobProvider.GetAttempts.ToList();
        attempts.Count.ShouldBe(3);
        attempts[0].ShouldBe(fix.StorageUrlA_Dual);
        attempts[1].ShouldBe(fix.StorageUrlA_Only);
        attempts[2].ShouldBe(fix.StorageUrlB_Dual);
    }

    [Fact]
    public async Task GetScreenshotContent_InactiveOrRemovedMembership_Returns403Forbidden()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Inactive member in wsA attempts to read capture -> 403 Forbidden
        var reqInactive = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Only}/content", InactiveUserA, fix.WsA);
        var respInactive = await client.SendAsync(reqInactive);
        respInactive.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        AssertNonStorablePrivateCachePolicy(respInactive);
        var bodyInactive = await respInactive.Content.ReadAsStringAsync();
        bodyInactive.ShouldContain("Tracker:NotWorkspaceMember");

        // 2. Remove UserAOnly from wsA in SQLite
        await WithUnitOfWorkAsync(async () =>
        {
            var member = await _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == fix.WsA && m.UserId == UserAOnly);
            if (member != null)
            {
                await _memberRepository.DeleteAsync(member, autoSave: true);
            }
        });

        // 3. Removed user attempts subsequent retrieval -> 403 Forbidden
        var reqRemoved = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Only}/content", UserAOnly, fix.WsA);
        var respRemoved = await client.SendAsync(reqRemoved);
        respRemoved.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        AssertNonStorablePrivateCachePolicy(respRemoved);
        var bodyRemoved = await respRemoved.Content.ReadAsStringAsync();
        bodyRemoved.ShouldContain("Tracker:NotWorkspaceMember");

        _blobProvider.GetCount.ShouldBe(0);
        _blobProvider.GetAttempts.ShouldBeEmpty();
    }

    [Fact]
    public async Task GetScreenshotContent_MissingBlobInStorage_Returns404NotFound()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // scB_MissingBlob has a valid SQL row for DualUser in wsB, but its blob was omitted from _blobProvider
        var req = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_MissingBlob}/content", DualUser, fix.WsB);
        var resp = await client.SendAsync(req);

        // Must yield mapped 404 Not Found, not successful 200 OK with empty 0 bytes
        resp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(resp);

        // Provider instrumentation confirms GetOrNullAsync was attempted for that key and returned null
        _blobProvider.GetCount.ShouldBe(1);
        _blobProvider.GetAttempts.ToList().ShouldBe(new[] { fix.StorageUrlB_MissingBlob });
    }

    [Fact]
    public async Task GetScreenshotContent_UnknownScreenshotId_Returns404NotFound_WithoutDisclosingContent()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var unknownId = _guidGenerator.Create();
        var req = CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{unknownId}/content", DualUser, fix.WsA);
        var resp = await client.SendAsync(req);

        resp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(resp);
        _blobProvider.GetCount.ShouldBe(0);
        _blobProvider.GetAttempts.ShouldBeEmpty();
    }

    [Fact]
    public async Task Preserves_All_Blob_Bytes_And_SQL_Sentinels_Across_Denied_Reads()
    {
        var fix = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Snapshot original blob keys and byte values
        var blobSnapshotBefore = _blobProvider.Blobs.ToDictionary(kvp => kvp.Key, kvp => (byte[])kvp.Value.Clone());
        blobSnapshotBefore.Count.ShouldBe(4);

        // Execute a battery of denied reads across all boundary categories
        // 1. Unauthenticated
        var r1 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", null, fix.WsA));
        r1.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        AssertNonStorablePrivateCachePolicy(r1);

        // 2. Missing workspace header
        var r2 = await client.SendAsync(CreateCustomRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Dual}/content", DualUser, null));
        r2.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        AssertNonStorablePrivateCachePolicy(r2);

        // 3. Spoofed foreign workspace header
        var r3 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", UserAOnly, fix.WsB));
        r3.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        AssertNonStorablePrivateCachePolicy(r3);

        // 4. Cross-workspace ID under wsA
        var r4 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Dual}/content", DualUser, fix.WsA));
        r4.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(r4);

        // 5. Member trying colleague's capture in wsB
        var r5 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScB_Other}/content", DualUser, fix.WsB));
        r5.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(r5);

        // 6. Inactive member in wsA
        var r6 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{fix.ScA_Only}/content", InactiveUserA, fix.WsA));
        r6.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        AssertNonStorablePrivateCachePolicy(r6);

        // 7. Unknown ID
        var r7 = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/activity/screenshot/{Guid.NewGuid()}/content", DualUser, fix.WsA));
        r7.StatusCode.ShouldBe(HttpStatusCode.NotFound);
        AssertNonStorablePrivateCachePolicy(r7);

        // Assert provider delete count is strictly 0 and save count is strictly 0
        _blobProvider.DeleteCount.ShouldBe(0);

        // Assert all seeded blob keys and bytes are preserved identically
        _blobProvider.Blobs.Count.ShouldBe(blobSnapshotBefore.Count);
        foreach (var (key, expectedBytes) in blobSnapshotBefore)
        {
            _blobProvider.Blobs.TryGetValue(key, out var currentBytes).ShouldBeTrue();
            currentBytes.ShouldNotBeNull();
            currentBytes.SequenceEqual(expectedBytes).ShouldBeTrue();
        }

        // Explicit target-workspace inspection: assert SQL sentinels in wsA and wsB remain intact
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(fix.WsA))
            {
                var wsMembersA = await _memberRepository.GetListAsync(m => m.WorkspaceId == fix.WsA);
                wsMembersA.Count.ShouldBe(3);

                var actsA = await _activityRepository.GetListAsync(a => a.WorkspaceId == fix.WsA);
                actsA.Count.ShouldBe(2);

                var scsA = await _screenshotRepository.GetListAsync(s => s.WorkspaceId == fix.WsA);
                scsA.Count.ShouldBe(2);
                scsA.Any(s => s.Id == fix.ScA_Dual).ShouldBeTrue();
                scsA.Any(s => s.Id == fix.ScA_Only).ShouldBeTrue();
            }

            using (_currentWorkspace.Change(fix.WsB))
            {
                var wsMembersB = await _memberRepository.GetListAsync(m => m.WorkspaceId == fix.WsB);
                wsMembersB.Count.ShouldBe(2);

                var actsB = await _activityRepository.GetListAsync(a => a.WorkspaceId == fix.WsB);
                actsB.Count.ShouldBe(3);

                var scsB = await _screenshotRepository.GetListAsync(s => s.WorkspaceId == fix.WsB);
                scsB.Count.ShouldBe(3);
                scsB.Any(s => s.Id == fix.ScB_Dual).ShouldBeTrue();
                scsB.Any(s => s.Id == fix.ScB_Other).ShouldBeTrue();
                scsB.Any(s => s.Id == fix.ScB_MissingBlob).ShouldBeTrue();
            }
        });
    }
}
