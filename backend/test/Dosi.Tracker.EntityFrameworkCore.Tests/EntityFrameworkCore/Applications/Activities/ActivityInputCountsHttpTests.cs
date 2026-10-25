using System;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Activities;

/// <summary>
/// The wire contract of a block's keyboard/mouse counts (<c>POST /api/app/activity</c>): an agent that cannot measure
/// input (no consent, a Wayland session) sends null or leaves the counts out and the block is stored as "not measured";
/// agents that always send numbers keep working, and a measured 0 stays 0.
/// </summary>
public class ActivityInputCountsHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Member = Guid.Parse("7b701e62-0953-4dd3-910b-dc6cc93ccb51");

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IGuidGenerator _guidGenerator;

    public ActivityInputCountsHttpTests()
    {
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
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
    }

    private async Task<(Guid WorkspaceId, Guid ProjectId)> SeedAsync()
    {
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("COUNTS-WS");
        var projectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            (await users.CreateAsync(new IdentityUser(Member, "counts.member", "counts.member@acme.test"))).Succeeded.ShouldBeTrue();

            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId, WorkspaceRoles.Owner))
            {
                await GetRequiredService<IRepository<Project, Guid>>()
                    .InsertAsync(new Project(projectId, workspaceId, "Counts Project"), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>()
                    .InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectId, Member, "Worker"), autoSave: true);
            }
        });
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        return (workspaceId, projectId);
    }

    private async Task<JsonElement> UploadAsync(Guid workspaceId, object block)
    {
        var response = await CreateHttpClient().SendAsync(CreateRequest(HttpMethod.Post, "/api/app/activity", Member, workspaceId, block));
        var body = await response.Content.ReadAsStringAsync();
        response.StatusCode.ShouldBe(HttpStatusCode.OK, body);
        return JsonDocument.Parse(body).RootElement.Clone();
    }

    private Task<Activity> StoredAsync(Guid workspaceId, Guid clientActivityId)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                return await _activityRepository.SingleAsync(a => a.ClientActivityId == clientActivityId);
            }
        });
    }

    [Fact]
    public async Task Null_Omitted_And_Numeric_Counts_Should_All_Be_Accepted_And_Stored_Honestly()
    {
        var (workspaceId, projectId) = await SeedAsync();
        var start = DateTime.UtcNow.AddHours(-3);

        var explicitNull = Guid.NewGuid();
        var nullDto = await UploadAsync(workspaceId, new
        {
            projectId,
            clientActivityId = explicitNull,
            startedAt = start,
            endedAt = start.AddMinutes(10),
            productivity = (int?)null,
            mouseClicks = (int?)null,
            keyboardHits = (int?)null,
            timeline = new[] { new { minute = 0, active = true, keyboardHits = (int?)null, mouseClicks = (int?)null } }
        });
        nullDto.GetProperty("mouseClicks").ValueKind.ShouldBe(JsonValueKind.Null);
        nullDto.GetProperty("keyboardHits").ValueKind.ShouldBe(JsonValueKind.Null);

        var omitted = Guid.NewGuid();
        await UploadAsync(workspaceId, new
        {
            projectId,
            clientActivityId = omitted,
            startedAt = start.AddMinutes(10),
            endedAt = start.AddMinutes(20)
        });

        var legacy = Guid.NewGuid();
        var legacyDto = await UploadAsync(workspaceId, new
        {
            projectId,
            clientActivityId = legacy,
            startedAt = start.AddMinutes(20),
            endedAt = start.AddMinutes(30),
            productivity = 40,
            mouseClicks = 0,
            keyboardHits = 57
        });
        legacyDto.GetProperty("mouseClicks").GetInt32().ShouldBe(0);
        legacyDto.GetProperty("keyboardHits").GetInt32().ShouldBe(57);

        var storedNull = await StoredAsync(workspaceId, explicitNull);
        storedNull.MouseClicks.ShouldBeNull();
        storedNull.KeyboardHits.ShouldBeNull();
        storedNull.TimelineJson.ShouldBe("[{\"Minute\":0,\"Active\":true}]");

        var storedOmitted = await StoredAsync(workspaceId, omitted);
        storedOmitted.MouseClicks.ShouldBeNull();
        storedOmitted.KeyboardHits.ShouldBeNull();

        var storedLegacy = await StoredAsync(workspaceId, legacy);
        storedLegacy.MouseClicks.ShouldBe(0);
        storedLegacy.KeyboardHits.ShouldBe(57);
    }
}
