using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Security;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
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

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Erasure;

/// <summary>
/// Deleting a workspace: only an owner, only by typing its name. It is gone at once for everyone — its members and their
/// agents are no longer members of it, its invitations stop working, its subscription is canceled and its name is free
/// again — and the erasure sweep purges its data, files included, once the grace period is over, leaving the invoices.
/// </summary>
// Runs a background sweep, whose named lock is process-wide (ABP's local lock): never alongside the other sweepers.
[Xunit.Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspaceDeletionHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("d2701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Manager = Guid.Parse("d2701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Member = Guid.Parse("d2701e62-0953-4dd3-910b-dc6cc93ccb03");

    private readonly ScriptedBlobProvider _blobProvider = new();
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public WorkspaceDeletionHttpTests()
    {
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

    private sealed record SeededWorkspace(Guid Id, Guid ProjectId, Guid ActivityId, string InvitationToken);

    /// <summary>A workspace with people, a project, a block with captures (and files), presence, a notification, a
    /// setting, a pending invitation, a subscription and an invoice.</summary>
    private async Task<SeededWorkspace> SeedWorkspaceAsync(string name)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            foreach (var (id, userName) in new[] { (Owner, "delete.owner"), (Manager, "delete.manager"), (Member, "delete.member") })
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

        var projectId = _guidGenerator.Create();
        var activityId = _guidGenerator.Create();
        var token = WorkspaceInvitation.NewToken();
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                var start = _clock.Now.AddHours(-3);
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, workspaceId, "Purged project"), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>()
                    .InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectId, Member), autoSave: true);
                await GetRequiredService<IRepository<Activity, Guid>>().InsertAsync(
                    new Activity(activityId, workspaceId, Member, projectId, _guidGenerator.Create(), start, start.AddMinutes(10)), autoSave: true);
                foreach (var kind in new[] { Screenshot.ScreenKind, Screenshot.ThumbKind })
                {
                    var blobName = $"activities/{activityId:D}/{kind}.jpg";
                    _blobProvider.Blobs[blobName] = new byte[] { 0xFF, 0xD8 };
                    await GetRequiredService<IRepository<Screenshot, Guid>>().InsertAsync(new Screenshot(
                        _guidGenerator.Create(), workspaceId, activityId, blobName, start.AddMinutes(5), 2, "image/jpeg", kind), autoSave: true);
                }

                await GetRequiredService<IRepository<AgentPresence, Guid>>().InsertAsync(
                    new AgentPresence(_guidGenerator.Create(), workspaceId, Member, Guid.NewGuid(), "active", _clock.Now), autoSave: true);
                await GetRequiredService<IRepository<Notification, Guid>>()
                    .InsertAsync(new Notification(_guidGenerator.Create(), workspaceId, Owner, "Hello"), autoSave: true);
                await GetRequiredService<WorkspaceSettingsManager>().SetMemberCaptureDeleteMinutesAsync(15);
                await GetRequiredService<IRepository<WorkspaceInvitation, Guid>>().InsertAsync(new WorkspaceInvitation(
                    _guidGenerator.Create(), workspaceId, "invitee@acme.test", WorkspaceRoles.Member, null, null, null,
                    WorkspaceInvitation.HashToken(token), _clock.Now.AddDays(7)), autoSave: true);

                var subscription = new Subscription(_guidGenerator.Create(), workspaceId, _guidGenerator.Create());
                subscription.Activate();
                await GetRequiredService<IRepository<Subscription, Guid>>().InsertAsync(subscription, autoSave: true);
                await GetRequiredService<IRepository<Invoice, Guid>>()
                    .InsertAsync(new Invoice(_guidGenerator.Create(), workspaceId, 30m, _clock.Now.AddDays(-20)), autoSave: true);
            }
        });

        return new SeededWorkspace(workspaceId, projectId, activityId, token);
    }

    private Task<int> CountAsync<TEntity>(Func<TEntity, bool> predicate) where TEntity : class, Volo.Abp.Domain.Entities.IEntity
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            using (GetRequiredService<IDataFilter>().Disable<ISoftDelete>())
            {
                return (await GetRequiredService<IRepository<TEntity>>().GetListAsync()).Count(predicate);
            }
        });
    }

    private Task<Workspace> GetWorkspaceAsync(Guid id)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<ISoftDelete>())
            {
                return await GetRequiredService<IRepository<Workspace, Guid>>().GetAsync(id);
            }
        });
    }

    private Task<HttpResponseMessage> DeleteAsync(HttpClient client, Guid workspaceId, Guid userId, string confirmation) =>
        client.SendAsync(CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{workspaceId}/delete", userId, null,
            new { confirmationName = confirmation }));

    private DataErasureWorker NewWorker(int graceDays) => new(
        GetRequiredService<AbpAsyncTimer>(),
        GetRequiredService<IServiceScopeFactory>(),
        Options.Create(new DataErasureOptions { WorkspaceGraceDays = graceDays, BatchSize = 1 }))
    {
        LazyServiceProvider = GetRequiredService<Volo.Abp.DependencyInjection.IAbpLazyServiceProvider>()
    };

    [Fact]
    public async Task Only_An_Owner_Deletes_A_Workspace_And_Only_By_Typing_Its_Name()
    {
        var workspace = await SeedWorkspaceAsync("Delete Me Inc");
        var client = CreateHttpClient();

        (await DeleteAsync(client, workspace.Id, Manager, "Delete Me Inc")).StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var mismatch = await DeleteAsync(client, workspace.Id, Owner, "Delete Me");
        mismatch.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        (await mismatch.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.WorkspaceDeleteConfirmationMismatch);
        (await GetWorkspaceAsync(workspace.Id)).IsDeleted.ShouldBeFalse();

        var deleted = await DeleteAsync(client, workspace.Id, Owner, "  delete me inc ");

        deleted.StatusCode.ShouldBe(HttpStatusCode.OK, await deleted.Content.ReadAsStringAsync());
        using var body = JsonDocument.Parse(await deleted.Content.ReadAsStringAsync());
        var deletionTime = body.RootElement.GetProperty("deletionTime").GetDateTime();
        body.RootElement.GetProperty("purgeAfter").GetDateTime().ShouldBe(deletionTime.AddDays(new DataErasureOptions().WorkspaceGraceDays));
        var row = await GetWorkspaceAsync(workspace.Id);
        row.IsDeleted.ShouldBeTrue();
        row.DeleterId.ShouldBe(Owner);
    }

    [Fact]
    public async Task A_Deleted_Workspace_Is_Gone_At_Once_For_Its_Members_Their_Agents_And_Its_Invitations()
    {
        var workspace = await SeedWorkspaceAsync("Gone Co");
        var client = CreateHttpClient();
        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/project", Member, workspace.Id))).StatusCode.ShouldBe(HttpStatusCode.OK);

        (await DeleteAsync(client, workspace.Id, Owner, "Gone Co")).StatusCode.ShouldBe(HttpStatusCode.OK);

        foreach (var userId in new[] { Owner, Manager, Member })
        {
            var refused = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/project", userId, workspace.Id));
            refused.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
            (await refused.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

            var mine = await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/workspaces/mine", userId, null));
            (await mine.Content.ReadAsStringAsync()).ShouldNotContain(workspace.Id.ToString());
        }

        (await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/invitations/{workspace.InvitationToken}", null, null)))
            .StatusCode.ShouldBe(HttpStatusCode.NotFound);
        (await CountAsync<WorkspaceInvitation>(i => i.WorkspaceId == workspace.Id)).ShouldBe(0);
        (await CountAsync<Subscription>(s => s.WorkspaceId == workspace.Id && s.Status == Subscription.CanceledStatus)).ShouldBe(1);
        (await DeleteAsync(client, workspace.Id, Owner, "Gone Co")).StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // The name is free again.
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/workspaces", Owner, null, new { name = "Gone Co" })))
            .StatusCode.ShouldBe(HttpStatusCode.OK);
    }

    [Fact]
    public async Task The_Sweep_Purges_A_Deleted_Workspace_After_Its_Grace_Period_And_Nothing_Else()
    {
        var deletedWorkspace = await SeedWorkspaceAsync("Purge Me");
        var keptWorkspace = await SeedWorkspaceAsync("Keep Me");
        var client = CreateHttpClient();
        (await DeleteAsync(client, deletedWorkspace.Id, Owner, "Purge Me")).StatusCode.ShouldBe(HttpStatusCode.OK);

        // Still within the grace period: nothing is purged.
        (await NewWorker(graceDays: 7).SweepAsync(ServiceProvider)).WorkspacesPurged.ShouldBe(0);
        (await CountAsync<Screenshot>(s => s.WorkspaceId == deletedWorkspace.Id)).ShouldBe(2);

        var result = await NewWorker(graceDays: 0).SweepAsync(ServiceProvider);

        result.WorkspacesPurged.ShouldBe(1);
        result.Unfinished.ShouldBe(0);
        var id = deletedWorkspace.Id;
        (await CountAsync<Screenshot>(s => s.WorkspaceId == id)).ShouldBe(0);
        _blobProvider.Blobs.Keys.ShouldNotContain(k => k.Contains(deletedWorkspace.ActivityId.ToString("D")));
        (await CountAsync<Activity>(a => a.WorkspaceId == id)).ShouldBe(0);
        (await CountAsync<Project>(p => p.WorkspaceId == id)).ShouldBe(0);
        (await CountAsync<ProjectMember>(m => m.WorkspaceId == id)).ShouldBe(0);
        (await CountAsync<WorkspaceMember>(m => m.WorkspaceId == id)).ShouldBe(0);
        (await CountAsync<AgentPresence>(p => p.WorkspaceId == id)).ShouldBe(0);
        (await CountAsync<Notification>(n => n.WorkspaceId == id)).ShouldBe(0);
        (await WithUnitOfWorkAsync(() => GetRequiredService<ISettingRepository>()
            .GetListAsync(WorkspaceSettingValueProvider.ProviderName, id.ToString()))).ShouldBeEmpty();
        (await CountAsync<Invoice>(i => i.WorkspaceId == id)).ShouldBe(1); // accounting records stay
        var purged = await GetWorkspaceAsync(id);
        purged.PurgedTime.ShouldNotBeNull();
        purged.Name.ShouldBe(Workspace.PurgedName(id));

        // The other workspace is untouched, and a second sweep has nothing left to do.
        var keptId = keptWorkspace.Id;
        (await CountAsync<Screenshot>(s => s.WorkspaceId == keptId)).ShouldBe(2);
        (await CountAsync<Activity>(a => a.WorkspaceId == keptId)).ShouldBe(1);
        (await CountAsync<WorkspaceMember>(m => m.WorkspaceId == keptId)).ShouldBe(3);
        (await NewWorker(graceDays: 0).SweepAsync(ServiceProvider)).WorkspacesPurged.ShouldBe(0);
    }

    [Fact]
    public async Task A_File_That_Cannot_Be_Deleted_Leaves_The_Workspace_Unfinished_Until_A_Later_Sweep()
    {
        var workspace = await SeedWorkspaceAsync("Store Down");
        var client = CreateHttpClient();
        (await DeleteAsync(client, workspace.Id, Owner, "Store Down")).StatusCode.ShouldBe(HttpStatusCode.OK);
        _blobProvider.BeforeDelete = (number, _) => number == 1 ? throw new InvalidOperationException("store down") : Task.CompletedTask;

        var first = await NewWorker(graceDays: 0).SweepAsync(ServiceProvider);

        first.WorkspacesPurged.ShouldBe(0);
        first.Unfinished.ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.WorkspaceId == workspace.Id)).ShouldBe(1);
        (await GetWorkspaceAsync(workspace.Id)).PurgedTime.ShouldBeNull();

        (await NewWorker(graceDays: 0).SweepAsync(ServiceProvider)).WorkspacesPurged.ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.WorkspaceId == workspace.Id)).ShouldBe(0);
        (await CountAsync<Activity>(a => a.WorkspaceId == workspace.Id)).ShouldBe(0);
    }

    [Fact]
    public async Task A_Workspace_That_Is_Not_Deleted_Is_Never_Purged()
    {
        var workspace = await SeedWorkspaceAsync("Alive Co");

        (await WithUnitOfWorkAsync(() => GetRequiredService<WorkspacePurger>().PurgeAsync(workspace.Id, 10))).ShouldBeFalse();
        (await WithUnitOfWorkAsync(() => GetRequiredService<WorkspacePurger>().PurgeAsync(Guid.NewGuid(), 10))).ShouldBeFalse();

        (await CountAsync<Activity>(a => a.WorkspaceId == workspace.Id)).ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.WorkspaceId == workspace.Id)).ShouldBe(2);
    }
}
