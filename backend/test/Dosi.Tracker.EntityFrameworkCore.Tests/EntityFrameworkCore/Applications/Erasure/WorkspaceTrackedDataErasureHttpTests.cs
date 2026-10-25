using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.BlobStoring;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Timing;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Erasure;

/// <summary>
/// Deleting tracked data over the hosted routes: owners and managers delete a member's block with its captures, or a
/// capture alone, giving a reason — the files go with them, every deletion is recorded, and a deleted block cannot be
/// uploaded again (410). Only an owner deletes an owner's data. Members delete their own captures within the
/// workspace's window after upload, and nobody else's.
/// </summary>
public class WorkspaceTrackedDataErasureHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("d1701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Manager = Guid.Parse("d1701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Member = Guid.Parse("d1701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid OtherMember = Guid.Parse("d1701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid Device = Guid.Parse("d1701e62-0953-4dd3-910b-dc6cc93ccb05");

    private readonly ScriptedBlobProvider _blobProvider = new();
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<ActivityErasure, Guid> _erasureRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public WorkspaceTrackedDataErasureHttpTests()
    {
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _erasureRepository = GetRequiredService<IRepository<ActivityErasure, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        UseScriptedBlobs(services);
    }

    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var connection = ServiceProvider.GetRequiredService<TrackerDbContext>().Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
            options.Configure(ctx => ctx.DbContextOptions.UseSqlite(connection)));
        services.AddAlwaysDisableUnitOfWorkTransaction();
        UseScriptedBlobs(services);
    }

    private void UseScriptedBlobs(IServiceCollection services)
    {
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private async Task<(Guid WorkspaceId, Guid ProjectId)> SeedWorkspaceAsync(string name)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            foreach (var (id, userName) in new[]
                     {
                         (Owner, "erase.owner"), (Manager, "erase.manager"), (Member, "erase.member"), (OtherMember, "erase.other")
                     })
            {
                if (await users.FindByIdAsync(id.ToString()) == null)
                {
                    (await users.CreateAsync(new IdentityUser(id, userName, $"{userName}@acme.test"))).Succeeded.ShouldBeTrue();
                }
            }
        });

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync(name);
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Manager, WorkspaceRoles.Manager, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(OtherMember, WorkspaceRoles.Member, workspaceId);

        var projectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                await GetRequiredService<IRepository<Project, Guid>>()
                    .InsertAsync(new Project(projectId, workspaceId, "Erasure project"), autoSave: true);
            }
        });

        return (workspaceId, projectId);
    }

    /// <summary>A block of <paramref name="userId"/> with a screen capture, its thumbnail and a webcam frame (rows and
    /// files), uploaded <paramref name="uploadedMinutesAgo"/> minutes ago.</summary>
    private async Task<(Guid ActivityId, Guid ClientActivityId, Guid ScreenId, Guid ThumbId, Guid WebcamId)> SeedBlockAsync(
        Guid workspaceId, Guid projectId, Guid userId, int startedHoursAgo = 2)
    {
        var activityId = _guidGenerator.Create();
        var clientActivityId = _guidGenerator.Create();
        var ids = (Screen: _guidGenerator.Create(), Thumb: _guidGenerator.Create(), Webcam: _guidGenerator.Create());
        var start = _clock.Now.AddHours(-startedHoursAgo);
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                await _activityRepository.InsertAsync(new Activity(
                    activityId, workspaceId, userId, projectId, clientActivityId, start, start.AddMinutes(10))
                {
                    Description = "Quarterly report",
                    DeviceId = Device
                }, autoSave: true);

                foreach (var (id, kind) in new[] { (ids.Screen, Screenshot.ScreenKind), (ids.Thumb, Screenshot.ThumbKind), (ids.Webcam, Screenshot.WebcamKind) })
                {
                    var blobName = $"activities/{activityId:D}/{kind}.jpg";
                    _blobProvider.Blobs[blobName] = new byte[] { 0xFF, 0xD8, 0xFF, 0xD9 };
                    await _screenshotRepository.InsertAsync(new Screenshot(
                        id, workspaceId, activityId, blobName, start.AddMinutes(5), 4, "image/jpeg", kind), autoSave: true);
                }
            }
        });

        return (activityId, clientActivityId, ids.Screen, ids.Thumb, ids.Webcam);
    }

    private Task<int> CountAsync<TEntity>(Func<TEntity, bool> predicate) where TEntity : class, Volo.Abp.Domain.Entities.IEntity
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return (await GetRequiredService<IRepository<TEntity>>().GetListAsync()).Count(predicate);
            }
        });
    }

    private static async Task<JsonElement> JsonAsync(HttpResponseMessage response)
    {
        using var document = JsonDocument.Parse(await response.Content.ReadAsStringAsync());
        return document.RootElement.Clone();
    }

    private static async Task ShouldFailWithAsync(HttpResponseMessage response, HttpStatusCode status, string code)
    {
        var body = await response.Content.ReadAsStringAsync();
        response.StatusCode.ShouldBe(status, body);
        body.ShouldContain(code);
    }

    [Fact]
    public async Task A_Manager_Deletes_A_Members_Block_With_Every_Capture_And_File_And_The_Deletion_Is_Recorded()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-BLOCK");
        var block = await SeedBlockAsync(workspaceId, projectId, Member);
        var kept = await SeedBlockAsync(workspaceId, projectId, Member, startedHoursAgo: 4);
        var client = CreateHttpClient();

        var response = await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{block.ActivityId}/delete",
            Manager, workspaceId, new { reason = "Tracked while on leave" }));

        response.StatusCode.ShouldBe(HttpStatusCode.OK, await response.Content.ReadAsStringAsync());
        var record = await JsonAsync(response);
        record.GetProperty("scope").GetString().ShouldBe(ActivityErasure.BlockScope);
        record.GetProperty("captureCount").GetInt32().ShouldBe(3);
        record.GetProperty("captureKinds").GetString().ShouldBe("screen,thumb,webcam");
        record.GetProperty("deletedBy").GetGuid().ShouldBe(Manager);
        record.GetProperty("userId").GetGuid().ShouldBe(Member);

        (await CountAsync<Activity>(a => a.Id == block.ActivityId)).ShouldBe(0);
        (await CountAsync<Screenshot>(s => s.ActivityId == block.ActivityId)).ShouldBe(0);
        _blobProvider.Blobs.Keys.ShouldNotContain(k => k.Contains(block.ActivityId.ToString("D")));
        (await CountAsync<Screenshot>(s => s.ActivityId == kept.ActivityId)).ShouldBe(3);
        _blobProvider.Blobs.Keys.Count(k => k.Contains(kept.ActivityId.ToString("D"))).ShouldBe(3);

        var erasure = (await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await _erasureRepository.GetListAsync(e => e.ActivityId == block.ActivityId);
            }
        })).Single();
        erasure.Reason.ShouldBe("Tracked while on leave");
        erasure.ClientActivityId.ShouldBe(block.ClientActivityId);
        erasure.CreatorId.ShouldBe(Manager);

        // Managers see the workspace's record of deletions; the member sees those of their own data.
        var managerList = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/activity/erasures", Manager, workspaceId)));
        managerList.GetProperty("totalCount").GetInt32().ShouldBe(1);
        var memberList = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/activity/erasures", Member, workspaceId)));
        memberList.GetProperty("items")[0].GetProperty("reason").GetString().ShouldBe("Tracked while on leave");
        var otherList = await JsonAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/activity/erasures", OtherMember, workspaceId)));
        otherList.GetProperty("totalCount").GetInt32().ShouldBe(0);
    }

    [Fact]
    public async Task A_Deleted_Block_Is_Refused_When_Its_Agent_Uploads_It_Again()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-REPLAY");
        var block = await SeedBlockAsync(workspaceId, projectId, Member);
        var client = CreateHttpClient();
        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{block.ActivityId}/delete",
            Owner, workspaceId, new { reason = "Duplicate block" }))).StatusCode.ShouldBe(HttpStatusCode.OK);

        var start = _clock.Now.AddHours(-2);
        var replay = CreateRequest(HttpMethod.Post, "/api/app/activity", Member, workspaceId, new
        {
            projectId,
            clientActivityId = block.ClientActivityId,
            startedAt = start,
            endedAt = start.AddMinutes(10),
            deviceId = Device
        });
        replay.Headers.Add(TestAuthHandler.DeviceHeaderName, Device.ToString());

        await ShouldFailWithAsync(await client.SendAsync(replay), HttpStatusCode.Gone, TrackerDomainErrorCodes.ActivityDeleted);
        (await CountAsync<Activity>(a => a.ClientActivityId == block.ClientActivityId)).ShouldBe(0);
    }

    [Fact]
    public async Task Only_Owners_And_Managers_Delete_Blocks_Only_With_A_Reason_And_Only_Owners_Touch_An_Owners_Data()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-RULES");
        var memberBlock = await SeedBlockAsync(workspaceId, projectId, Member);
        var ownerBlock = await SeedBlockAsync(workspaceId, projectId, Owner);
        var client = CreateHttpClient();

        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{memberBlock.ActivityId}/delete",
            Member, workspaceId, new { reason = "My own block" }))).StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{memberBlock.ActivityId}/delete",
            Manager, workspaceId, new { reason = "" }))).StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        await ShouldFailWithAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{ownerBlock.ActivityId}/delete",
                Manager, workspaceId, new { reason = "Not mine to delete" })),
            HttpStatusCode.Forbidden, TrackerDomainErrorCodes.RoleNotAllowed);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{Guid.NewGuid()}/delete",
            Owner, workspaceId, new { reason = "Does not exist" }))).StatusCode.ShouldBe(HttpStatusCode.NotFound);

        (await CountAsync<Activity>(a => a.Id == memberBlock.ActivityId || a.Id == ownerBlock.ActivityId)).ShouldBe(2);

        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{ownerBlock.ActivityId}/delete",
            Owner, workspaceId, new { reason = "My own mistake" }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await CountAsync<Activity>(a => a.Id == ownerBlock.ActivityId)).ShouldBe(0);
    }

    [Fact]
    public async Task A_Block_Of_Another_Workspace_Is_Not_Found()
    {
        var (workspaceA, projectA) = await SeedWorkspaceAsync("ERASE-ISO-A");
        var (workspaceB, _) = await SeedWorkspaceAsync("ERASE-ISO-B");
        var block = await SeedBlockAsync(workspaceA, projectA, Member);
        var client = CreateHttpClient();

        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{block.ActivityId}/delete",
            Owner, workspaceB, new { reason = "Wrong workspace" }))).StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{block.ScreenId}/delete",
            Owner, workspaceB, new { reason = "Wrong workspace" }))).StatusCode.ShouldBe(HttpStatusCode.NotFound);

        (await CountAsync<Screenshot>(s => s.ActivityId == block.ActivityId)).ShouldBe(3);
    }

    [Fact]
    public async Task A_Member_Deletes_Their_Own_Fresh_Capture_With_Its_Thumbnail_But_The_Block_Stays()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-OWN");
        var block = await SeedBlockAsync(workspaceId, projectId, Member);
        var client = CreateHttpClient();

        var response = await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{block.ThumbId}/delete",
            Member, workspaceId, new { }));

        response.StatusCode.ShouldBe(HttpStatusCode.OK, await response.Content.ReadAsStringAsync());
        var record = await JsonAsync(response);
        record.GetProperty("scope").GetString().ShouldBe(ActivityErasure.CaptureScope);
        record.GetProperty("captureKinds").GetString().ShouldBe("screen,thumb");
        record.GetProperty("reason").ValueKind.ShouldBe(JsonValueKind.Null);

        (await CountAsync<Activity>(a => a.Id == block.ActivityId)).ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.ActivityId == block.ActivityId)).ShouldBe(1); // the webcam frame
        _blobProvider.Blobs.Keys.Where(k => k.Contains(block.ActivityId.ToString("D")))
            .ShouldBe(new[] { $"activities/{block.ActivityId:D}/webcam.jpg" });
    }

    [Fact]
    public async Task A_Member_Cannot_Delete_A_Capture_After_The_Window_Or_When_The_Owner_Turned_It_Off_Or_Someone_Elses()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-WINDOW");
        var own = await SeedBlockAsync(workspaceId, projectId, Member);
        var others = await SeedBlockAsync(workspaceId, projectId, OtherMember, startedHoursAgo: 3);
        var client = CreateHttpClient();

        await ShouldFailWithAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{others.ScreenId}/delete",
                Member, workspaceId, new { reason = "not mine" })),
            HttpStatusCode.NotFound, "There is no entity Screenshot");

        // The owner turns self-deletion off.
        (await client.SendAsync(CreateRequest(HttpMethod.Put, "/api/app/workspace/settings", Owner, workspaceId, new
        {
            newProjectDefaults = new { intervalMinutes = 10, idleThresholdMinutes = 5 },
            memberCaptureDeleteMinutes = 0
        }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        await ShouldFailWithAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{own.WebcamId}/delete",
                Member, workspaceId, new { })),
            HttpStatusCode.Forbidden, TrackerDomainErrorCodes.CaptureDeleteNotAllowed);

        // A window of 10 minutes is over for a capture uploaded an hour ago.
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                await GetRequiredService<WorkspaceSettingsManager>().SetMemberCaptureDeleteMinutesAsync(10);
                var webcam = await _screenshotRepository.GetAsync(own.WebcamId);
                ObjectHelper.TrySetProperty(webcam, s => s.CreationTime, () => _clock.Now.AddHours(-1));
                await _screenshotRepository.UpdateAsync(webcam, autoSave: true);
            }
        });
        await ShouldFailWithAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{own.WebcamId}/delete",
                Member, workspaceId, new { })),
            HttpStatusCode.Forbidden, TrackerDomainErrorCodes.CaptureDeleteNotAllowed);

        (await CountAsync<Screenshot>(s => s.ActivityId == own.ActivityId || s.ActivityId == others.ActivityId)).ShouldBe(6);
    }

    [Fact]
    public async Task A_Manager_Deletes_A_Members_Capture_Only_With_A_Reason()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-CAPTURE");
        var block = await SeedBlockAsync(workspaceId, projectId, Member);
        var client = CreateHttpClient();

        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{block.WebcamId}/delete",
            Manager, workspaceId, new { }))).StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        var response = await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/capture/{block.WebcamId}/delete",
            Manager, workspaceId, new { reason = "Webcam caught a family member" }));
        response.StatusCode.ShouldBe(HttpStatusCode.OK);
        (await JsonAsync(response)).GetProperty("captureKinds").GetString().ShouldBe("webcam");

        (await CountAsync<Screenshot>(s => s.ActivityId == block.ActivityId)).ShouldBe(2);
        _blobProvider.Blobs.ContainsKey($"activities/{block.ActivityId:D}/webcam.jpg").ShouldBeFalse();
    }

    [Fact]
    public async Task A_File_That_Cannot_Be_Deleted_Fails_The_Request_And_Leaves_The_Capture_To_Delete_Again()
    {
        var (workspaceId, projectId) = await SeedWorkspaceAsync("ERASE-STORE-DOWN");
        var block = await SeedBlockAsync(workspaceId, projectId, Member);
        var client = CreateHttpClient();
        _blobProvider.BeforeDelete = (_, _) => throw new InvalidOperationException("the object store is down");

        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{block.ActivityId}/delete",
            Owner, workspaceId, new { reason = "Try while the store is down" }))).IsSuccessStatusCode.ShouldBeFalse();
        (await CountAsync<Activity>(a => a.Id == block.ActivityId)).ShouldBe(1);
        (await CountAsync<ActivityErasure>(e => e.ActivityId == block.ActivityId)).ShouldBe(0);

        _blobProvider.BeforeDelete = null;
        (await client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/activity/{block.ActivityId}/delete",
            Owner, workspaceId, new { reason = "Again, once the store is back" }))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await CountAsync<Screenshot>(s => s.ActivityId == block.ActivityId)).ShouldBe(0);
    }
}
