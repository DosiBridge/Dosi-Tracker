using System;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
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
/// Activity (time block) retention, end to end: the owner-managed policy over the hosted routes
/// (<c>/api/app/workspaces/{id}/activity-retention</c>), how it reads (only the workspace's own, well-formed value turns
/// it on), and the worker that enforces it — only for opted-in, active workspaces, never touching blocks that still have
/// captures or lie inside the upload window, and stopped entirely by the operator switch.
/// </summary>
public class WorkspaceActivityRetentionPolicyHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb61");
    private static readonly Guid Member = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb62");
    private static readonly Guid Outsider = Guid.Parse("8c701e62-0953-4dd3-910b-dc6cc93ccb63");

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly WorkspaceSettingsManager _settings;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public WorkspaceActivityRetentionPolicyHttpTests()
    {
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _settings = GetRequiredService<WorkspaceSettingsManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
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
    }

    private async Task<Guid> SeedWorkspaceAsync(string name)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            foreach (var (id, userName) in new[] { (Owner, "retention.owner"), (Member, "retention.member"), (Outsider, "retention.outsider") })
            {
                if (await users.FindByIdAsync(id.ToString()) == null)
                {
                    (await users.CreateAsync(new IdentityUser(id, userName, $"{userName}@acme.test"))).Succeeded.ShouldBeTrue();
                }
            }
        });

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync(name);
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        return workspaceId;
    }

    private async Task<Guid> SeedBlockAsync(Guid workspaceId, int daysAgo, bool withCapture = false)
    {
        var activityId = _guidGenerator.Create();
        var start = _clock.Now.AddDays(-daysAgo);
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                var projectId = _guidGenerator.Create();
                await GetRequiredService<IRepository<Project, Guid>>()
                    .InsertAsync(new Project(projectId, workspaceId, $"Project {projectId:N}"), autoSave: true);
                await _activityRepository.InsertAsync(new Activity(
                    activityId, workspaceId, Owner, projectId, _guidGenerator.Create(), start, start.AddMinutes(10)), autoSave: true);

                if (withCapture)
                {
                    await _screenshotRepository.InsertAsync(new Screenshot(
                        _guidGenerator.Create(), workspaceId, activityId, $"activities/{activityId}/screen.jpg",
                        start.AddMinutes(5), 1024, "image/jpeg", Screenshot.ScreenKind), autoSave: true);
                }
            }
        });

        return activityId;
    }

    private Task<bool> ExistsAsync(Guid activityId)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await _activityRepository.FindAsync(activityId) != null;
            }
        });
    }

    private Task SetPolicyAsync(Guid workspaceId, bool enabled, int days) =>
        WithUnitOfWorkAsync(() => _settings.SetActivityRetentionPolicyAsync(workspaceId, enabled, days));

    private ActivityRetentionWorker NewWorker(ActivityRetentionOptions? options = null) => new(
        GetRequiredService<AbpAsyncTimer>(),
        GetRequiredService<IServiceScopeFactory>(),
        Options.Create(options ?? new ActivityRetentionOptions()));

    [Fact]
    public async Task Members_Should_Read_A_Default_Off_Policy_And_Only_Owners_Change_It()
    {
        var workspaceId = await SeedWorkspaceAsync("ACT-RET-POLICY");
        var client = CreateHttpClient();
        var route = $"/api/app/workspaces/{workspaceId}/activity-retention";

        var read = await client.SendAsync(CreateRequest(HttpMethod.Get, route, Member, workspaceId));
        read.StatusCode.ShouldBe(HttpStatusCode.OK);
        using (var body = JsonDocument.Parse(await read.Content.ReadAsStringAsync()))
        {
            body.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeFalse();
            body.RootElement.GetProperty("retentionDays").GetInt32().ShouldBe(ActivityRetentionLimits.DefaultDays);
            body.RootElement.GetProperty("isEnforced").GetBoolean().ShouldBeTrue();
        }

        // Capture retention reports the same way whether this server enforces policies.
        var capture = await client.SendAsync(CreateRequest(HttpMethod.Get, $"/api/app/workspaces/{workspaceId}/capture-retention", Member, workspaceId));
        using (var body = JsonDocument.Parse(await capture.Content.ReadAsStringAsync()))
        {
            body.RootElement.GetProperty("isEnforced").GetBoolean().ShouldBeTrue();
        }

        (await client.SendAsync(CreateRequest(HttpMethod.Get, route, Outsider, null))).StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await client.SendAsync(CreateRequest(HttpMethod.Put, route, Member, workspaceId, new { isEnabled = true, retentionDays = 120 })))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // Inside the upload window (and the minimum a pay period needs): refused.
        (await client.SendAsync(CreateRequest(HttpMethod.Put, route, Owner, workspaceId, new { isEnabled = true, retentionDays = 30 })))
            .StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        var update = await client.SendAsync(CreateRequest(HttpMethod.Put, route, Owner, workspaceId, new { isEnabled = true, retentionDays = 120 }));
        update.StatusCode.ShouldBe(HttpStatusCode.OK);

        (await WithUnitOfWorkAsync(() => _settings.GetActivityRetentionPolicyAsync(workspaceId))).ShouldBe((true, 120));
    }

    [Fact]
    public async Task A_Policy_Should_Only_Ever_Come_From_The_Workspaces_Own_Well_Formed_Value()
    {
        var workspaceId = await SeedWorkspaceAsync("ACT-RET-READ");
        var settingManager = GetRequiredService<ISettingManager>();

        // A platform-wide value never turns deletion on for a workspace.
        await WithUnitOfWorkAsync(async () =>
        {
            await settingManager.SetGlobalAsync(TrackerSettings.Workspace.ActivityRetention.IsEnabled, "True");
            await settingManager.SetGlobalAsync(TrackerSettings.Workspace.ActivityRetention.RetentionDays, "120");
        });
        (await WithUnitOfWorkAsync(() => _settings.GetActivityRetentionPolicyAsync(workspaceId)))
            .ShouldBe((false, ActivityRetentionLimits.DefaultDays));

        // Malformed or out-of-range values read as off.
        foreach (var (enabled, days) in new[] { ("yes", "120"), ("True", "abc"), ("True", "30"), ("True", "99999") })
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await settingManager.SetAsync(TrackerSettings.Workspace.ActivityRetention.IsEnabled, enabled,
                    WorkspaceSettingValueProvider.ProviderName, workspaceId.ToString());
                await settingManager.SetAsync(TrackerSettings.Workspace.ActivityRetention.RetentionDays, days,
                    WorkspaceSettingValueProvider.ProviderName, workspaceId.ToString());
            });

            (await WithUnitOfWorkAsync(() => _settings.GetActivityRetentionPolicyAsync(workspaceId))).IsEnabled.ShouldBeFalse();
        }
    }

    [Fact]
    public async Task The_Worker_Should_Delete_Only_Expired_Blocks_Of_Opted_In_Active_Workspaces()
    {
        var optedIn = await SeedWorkspaceAsync("ACT-RET-ON");
        var optedOut = await SeedWorkspaceAsync("ACT-RET-OFF");
        var archived = await SeedWorkspaceAsync("ACT-RET-ARCHIVED");
        await SetPolicyAsync(optedIn, true, 90);
        await SetPolicyAsync(archived, true, 90);
        await WithUnitOfWorkAsync(async () =>
        {
            var workspace = await _workspaceRepository.GetAsync(archived);
            workspace.Archive(Owner, _clock.Now);
            await _workspaceRepository.UpdateAsync(workspace, autoSave: true);
        });

        var expired = await SeedBlockAsync(optedIn, daysAgo: 200);
        var expiredWithCapture = await SeedBlockAsync(optedIn, daysAgo: 200, withCapture: true);
        var recent = await SeedBlockAsync(optedIn, daysAgo: 30);
        var otherWorkspace = await SeedBlockAsync(optedOut, daysAgo: 200);
        var archivedBlock = await SeedBlockAsync(archived, daysAgo: 200);

        var worker = NewWorker();
        var result = await worker.SweepAsync(ServiceProvider);

        result.Skipped.ShouldBeFalse();
        result.TotalDeleted.ShouldBe(1);
        result.TotalPreserved.ShouldBe(1);
        result.Errors.ShouldBe(0);
        (await ExistsAsync(expired)).ShouldBeFalse();
        (await ExistsAsync(expiredWithCapture)).ShouldBeTrue(); // capture retention removes its captures first
        (await ExistsAsync(recent)).ShouldBeTrue();
        (await ExistsAsync(otherWorkspace)).ShouldBeTrue();
        (await ExistsAsync(archivedBlock)).ShouldBeTrue();
    }

    [Fact]
    public async Task The_Operator_Switch_Should_Stop_Every_Sweep_And_The_Workers_Should_Run_At_Start_Up()
    {
        var workspaceId = await SeedWorkspaceAsync("ACT-RET-SWITCH");
        await SetPolicyAsync(workspaceId, true, 90);
        var expired = await SeedBlockAsync(workspaceId, daysAgo: 200);

        var result = await NewWorker(new ActivityRetentionOptions { IsEnabled = false }).SweepAsync(ServiceProvider);

        result.Skipped.ShouldBeTrue();
        result.SkipReason.ShouldBe("Disabled by configuration");
        (await ExistsAsync(expired)).ShouldBeTrue();

        // A restart (deploy, scaling, crash) must not postpone a sweep by a whole period.
        var activityTimer = GetRequiredService<AbpAsyncTimer>();
        _ = new ActivityRetentionWorker(activityTimer, GetRequiredService<IServiceScopeFactory>(),
            Options.Create(new ActivityRetentionOptions { PeriodHours = 6 }));
        activityTimer.RunOnStart.ShouldBeTrue();
        activityTimer.Period.ShouldBe((int)TimeSpan.FromHours(6).TotalMilliseconds);

        var captureTimer = GetRequiredService<AbpAsyncTimer>();
        _ = new CaptureRetentionWorker(captureTimer, GetRequiredService<IServiceScopeFactory>(),
            Options.Create(new CaptureRetentionOptions()));
        captureTimer.RunOnStart.ShouldBeTrue();
    }
}
