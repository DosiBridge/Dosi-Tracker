using System;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.AiAssist.Fakes;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Controllers;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

/// <summary>
/// The AI cards over the hosted HTTP pipeline with the real workspace resolution and permissions: each refusal maps to
/// its status with <c>error.data</c>, and a served card is generated, cached and rated.
/// </summary>
public class WorkspaceAiInsightsHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccc71");
    private static readonly Guid Member = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccc72");

    private static readonly DateTime Day = DateTime.SpecifyKind(DateTime.UtcNow.Date.AddDays(-3), DateTimeKind.Utc);

    private readonly FakeAiChatClientProvider _provider = new();
    private int? _userDailyRequests;

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
        services.Replace(ServiceDescriptor.Singleton<IAiChatClientProvider>(_provider));
        services.PostConfigure<AiOptions>(options =>
        {
            options.Quotas.Default.UserPerMinute = 100;
            if (_userDailyRequests.HasValue)
            {
                options.Quotas.Default.UserDailyRequests = _userDailyRequests;
            }
        });
    }

    private async Task<Guid> SeedAsync()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            (await users.CreateAsync(new IdentityUser(Owner, "insights.owner", "insights.owner@acme.test"))).Succeeded.ShouldBeTrue();
            (await users.CreateAsync(new IdentityUser(Member, "insights.member", "insights.member@acme.test"))).Succeeded.ShouldBeTrue();
        });

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("AI Insights Co");
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);

        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                var projectId = Guid.NewGuid();
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, workspaceId, "Website"), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                    new ProjectMember(Guid.NewGuid(), workspaceId, projectId, Member, ProjectMemberRoles.Worker), autoSave: true);
                var start = Day.AddHours(9);
                await GetRequiredService<IRepository<Activity, Guid>>().InsertAsync(
                    new Activity(Guid.NewGuid(), workspaceId, Member, projectId, Guid.NewGuid(), start, start.AddMinutes(90)) { Productivity = 65 },
                    autoSave: true);
            }
        });

        return workspaceId;
    }

    private static object Enable() => new
    {
        isEnabled = true,
        allowManagers = true,
        allowMemberSelfService = true,
        allowWindowTitles = false,
        allowNotes = false,
        acknowledgedDisclosureVersion = "1.0"
    };

    private static object Week(int days = 7) => new
    {
        from = Day.AddDays(1 - days),
        to = Day.AddDays(1),
        scope = "team",
        timeZone = "UTC"
    };

    private static async Task<JsonElement> ErrorAsync(HttpResponseMessage response, HttpStatusCode expected, string code)
    {
        var body = await response.Content.ReadAsStringAsync();
        response.StatusCode.ShouldBe(expected, body);
        using var document = JsonDocument.Parse(body);
        var error = document.RootElement.GetProperty("error");
        error.GetProperty("code").GetString().ShouldBe(code);
        return error.Clone();
    }

    private async Task EnableAsync(HttpClient client, Guid workspaceId)
    {
        var enabled = await client.SendAsync(CreateRequest(HttpMethod.Put, "/api/app/ai-assist/settings", Owner, workspaceId, Enable()));
        enabled.StatusCode.ShouldBe(HttpStatusCode.OK, await enabled.Content.ReadAsStringAsync());
    }

    [Fact]
    public async Task A_Server_Without_A_Provider_Should_Answer_503()
    {
        var workspaceId = await SeedAsync();
        _provider.State = AiProviderState.MissingApiKey;
        var client = CreateHttpClient();

        var error = await ErrorAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/period-summary", Owner, workspaceId, Week())),
            HttpStatusCode.ServiceUnavailable, TrackerDomainErrorCodes.AiNotConfigured);

        error.GetProperty("data").GetProperty("featureId").GetString().ShouldBe(AiFeatureIds.PeriodSummaryId);
        _provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Workspace_That_Did_Not_Opt_In_Should_Answer_409_Off()
    {
        var workspaceId = await SeedAsync();
        var client = CreateHttpClient();

        var error = await ErrorAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/member-day-summary", Member, workspaceId,
                new { date = Day.ToString("yyyy-MM-dd"), timeZone = "UTC" })),
            HttpStatusCode.Conflict, TrackerDomainErrorCodes.AiDisabled);

        error.GetProperty("data").GetProperty("reason").GetString().ShouldBe("off");
        _provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Range_Too_Long_Should_Answer_400_With_The_Cap()
    {
        var workspaceId = await SeedAsync();
        var client = CreateHttpClient();
        await EnableAsync(client, workspaceId);

        var error = await ErrorAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/period-summary", Owner, workspaceId, Week(40))),
            HttpStatusCode.BadRequest, TrackerDomainErrorCodes.AiRangeTooLarge);

        error.GetProperty("data").GetProperty("maxDays").GetInt32().ShouldBe(AiAssistConsts.MaxPeriodDays);
        error.GetProperty("message").GetString().ShouldBe("Choose 31 days or fewer for AI summaries.");
    }

    [Fact]
    public async Task A_Used_Up_Allowance_Should_Answer_429_With_Its_Scope_And_Renewal()
    {
        var workspaceId = await SeedAsync();
        _userDailyRequests = 0;
        var client = CreateHttpClient();
        await EnableAsync(client, workspaceId);

        var error = await ErrorAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/period-summary", Owner, workspaceId, Week())),
            HttpStatusCode.TooManyRequests, TrackerDomainErrorCodes.AiQuotaExceeded);

        var data = error.GetProperty("data");
        data.GetProperty("scope").GetString().ShouldBe("user-day");
        data.GetProperty("resetsAt").GetDateTime().ShouldBeGreaterThan(DateTime.UtcNow.AddMinutes(-1));
        _provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task A_Card_Should_Be_Generated_Then_Cached_And_Rated()
    {
        var workspaceId = await SeedAsync();
        var client = CreateHttpClient();
        await EnableAsync(client, workspaceId);
        _provider.Client.EnqueueJson(JsonSerializer.Serialize(new
        {
            headline = "The team tracked {F1}.",
            insights = Array.Empty<object>(),
            nextSteps = Array.Empty<object>(),
            caveats = Array.Empty<string>()
        }));

        Guid resultId;
        foreach (var expected in new[] { AiResultStatuses.Generated, AiResultStatuses.Cached })
        {
            var response = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/period-summary", Owner, workspaceId, Week()));
            var body = await response.Content.ReadAsStringAsync();
            response.StatusCode.ShouldBe(HttpStatusCode.OK, body);
            using var document = JsonDocument.Parse(body);
            document.RootElement.GetProperty("meta").GetProperty("status").GetString().ShouldBe(expected);
            document.RootElement.GetProperty("headline").GetString().ShouldBe("The team tracked 1 h 30 min.");
            resultId = document.RootElement.GetProperty("meta").GetProperty("resultId").GetGuid();

            var rated = await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/feedback", Owner, workspaceId,
                new { resultId, rating = "up", reason = "other" }));
            rated.StatusCode.ShouldBe(HttpStatusCode.NoContent, await rated.Content.ReadAsStringAsync());

            // Someone else's result is not theirs to rate.
            (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/feedback", Member, workspaceId,
                new { resultId, rating = "down" }))).StatusCode.ShouldBe(HttpStatusCode.NotFound);
        }

        _provider.Client.Calls.Count.ShouldBe(1);
        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/feedback", Owner, workspaceId,
            new { resultId = Guid.NewGuid(), rating = "sideways" }))).StatusCode.ShouldBe(HttpStatusCode.BadRequest);
    }

    [Fact]
    public async Task A_Member_Should_Be_Refused_Team_Cards_With_403()
    {
        var workspaceId = await SeedAsync();
        var client = CreateHttpClient();
        await EnableAsync(client, workspaceId);

        (await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/period-summary", Member, workspaceId, Week())))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        await ErrorAsync(
            await client.SendAsync(CreateRequest(HttpMethod.Post, "/api/app/ai-assist/report-explanation", Member, workspaceId,
                new { reportSlug = "weekly", from = Day, to = Day.AddDays(1) })),
            HttpStatusCode.Forbidden, TrackerDomainErrorCodes.AiFeatureDisabled);
        _provider.Client.Calls.ShouldBeEmpty();
    }
}
