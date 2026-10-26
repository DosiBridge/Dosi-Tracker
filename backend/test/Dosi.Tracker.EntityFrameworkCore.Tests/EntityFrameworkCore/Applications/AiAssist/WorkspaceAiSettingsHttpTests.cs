using System;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
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
/// AI assist status and settings over the hosted HTTP pipeline, with the real workspace resolution and the real
/// role-based permissions: every member reads the status, only the owner changes the settings (with the disclosure
/// acknowledged), a foreign workspace is refused before any AI code runs, and the platform part is closed to
/// workspace users.
/// </summary>
public class WorkspaceAiSettingsHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid Owner = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccb71");
    private static readonly Guid Manager = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccb72");
    private static readonly Guid Member = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccb73");
    private static readonly Guid Client = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccb74");
    private static readonly Guid Outsider = Guid.Parse("9a701e62-0953-4dd3-910b-dc6cc93ccb75");

    private readonly FakeAiChatClientProvider _provider = new();

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
    }

    private async Task<(Guid Workspace, Guid OtherWorkspace)> SeedAsync()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IdentityUserManager>();
            foreach (var (id, userName) in new[]
                     {
                         (Owner, "ai.owner"), (Manager, "ai.manager"), (Member, "ai.member"), (Client, "ai.client"),
                         (Outsider, "ai.outsider")
                     })
            {
                (await users.CreateAsync(new IdentityUser(id, userName, $"{userName}@acme.test"))).Succeeded.ShouldBeTrue();
            }
        });

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("AI HTTP Co");
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Manager, WorkspaceRoles.Manager, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Client, WorkspaceRoles.Member, workspaceId);

        var otherId = await ServiceProvider.CreateWorkspaceAsync("AI HTTP Other Co");
        await ServiceProvider.AddWorkspaceMemberAsync(Outsider, WorkspaceRoles.Owner, otherId);

        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                var projectId = Guid.NewGuid();
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, workspaceId, "Client view"), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                    new ProjectMember(Guid.NewGuid(), workspaceId, projectId, Client, ProjectMemberRoles.Client), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                    new ProjectMember(Guid.NewGuid(), workspaceId, projectId, Member, ProjectMemberRoles.Worker), autoSave: true);
            }
        });

        return (workspaceId, otherId);
    }

    private static object Settings(bool enabled = true, string? ack = "1.0") => new
    {
        isEnabled = enabled,
        allowManagers = true,
        allowMemberSelfService = true,
        allowWindowTitles = false,
        allowNotes = false,
        acknowledgedDisclosureVersion = ack
    };

    private static async Task<JsonDocument> ReadAsync(HttpResponseMessage response, HttpStatusCode expected)
    {
        var body = await response.Content.ReadAsStringAsync();
        response.StatusCode.ShouldBe(expected, body);
        return JsonDocument.Parse(body);
    }

    private static JsonElement Feature(JsonDocument status, string featureId) =>
        status.RootElement.GetProperty("features").EnumerateArray().Single(f => f.GetProperty("featureId").GetString() == featureId);

    [Fact]
    public async Task Every_Role_Should_Read_The_Status_And_See_What_It_Allows_Them()
    {
        var (workspaceId, _) = await SeedAsync();
        var client = CreateHttpClient();

        // Off until the owner turns it on.
        using (var off = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Member, workspaceId)), HttpStatusCode.OK))
        {
            off.RootElement.GetProperty("available").GetBoolean().ShouldBeFalse();
            off.RootElement.GetProperty("reason").GetString().ShouldBe("workspace-disabled");
        }

        using (var enabled = await ReadAsync(await client.SendAsync(
                   CreateRequest(HttpMethod.Put, "/api/app/ai-assist/settings", Owner, workspaceId, Settings())), HttpStatusCode.OK))
        {
            enabled.RootElement.GetProperty("isEnabled").GetBoolean().ShouldBeTrue();
            enabled.RootElement.GetProperty("consentVersion").GetString().ShouldBe("1.0");
            enabled.RootElement.GetProperty("enabledByName").GetString().ShouldBe("ai.owner");
            enabled.RootElement.GetProperty("disclosure").GetProperty("neverSent").GetArrayLength().ShouldBeGreaterThan(0);
        }

        using (var owner = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Owner, workspaceId)), HttpStatusCode.OK))
        {
            owner.RootElement.GetProperty("available").GetBoolean().ShouldBeTrue();
            owner.RootElement.GetProperty("canManage").GetBoolean().ShouldBeTrue();
            owner.RootElement.GetProperty("providerDisplayName").GetString().ShouldBe("OpenAI");
            owner.RootElement.GetProperty("quota").GetProperty("requestsUsed").GetInt32().ShouldBe(0);
            Feature(owner, AiFeatureIds.PeriodSummaryId).GetProperty("scopes").EnumerateArray().Select(s => s.GetString())
                .ShouldBe(new[] { "team", "self" });
        }

        using (var manager = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Manager, workspaceId)), HttpStatusCode.OK))
        {
            manager.RootElement.GetProperty("canManage").GetBoolean().ShouldBeFalse();
            Feature(manager, AiFeatureIds.ReportExplanationId).GetProperty("allowed").GetBoolean().ShouldBeTrue();
            Feature(manager, AiFeatureIds.MemberDaySummaryId).GetProperty("scopes").EnumerateArray().Select(s => s.GetString())
                .ShouldBe(new[] { "member", "self" });
        }

        // The member's real permissions (no ViewAll, no Reporting): their own data only.
        using (var member = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Member, workspaceId)), HttpStatusCode.OK))
        {
            member.RootElement.GetProperty("available").GetBoolean().ShouldBeTrue();
            member.RootElement.GetProperty("quota").ValueKind.ShouldBe(JsonValueKind.Null);
            var report = Feature(member, AiFeatureIds.ReportExplanationId);
            report.GetProperty("allowed").GetBoolean().ShouldBeFalse();
            report.GetProperty("reason").GetString().ShouldBe("role");
            Feature(member, AiFeatureIds.PeriodSummaryId).GetProperty("scopes").EnumerateArray().Select(s => s.GetString())
                .ShouldBe(new[] { "self" });
        }

        using (var clientOnly = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Client, workspaceId)), HttpStatusCode.OK))
        {
            clientOnly.RootElement.GetProperty("available").GetBoolean().ShouldBeFalse();
            clientOnly.RootElement.GetProperty("reason").GetString().ShouldBe("role-not-allowed");
            clientOnly.RootElement.GetProperty("features").EnumerateArray().ShouldAllBe(f => !f.GetProperty("allowed").GetBoolean());
        }

        // Members can read their own usage; only owners and managers the settings.
        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/usage", Member, workspaceId))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/settings", Manager, workspaceId))).StatusCode.ShouldBe(HttpStatusCode.OK);
        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/settings", Member, workspaceId))).StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        _provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task Only_The_Owner_Should_Change_The_Settings_And_Only_With_The_Disclosure_Acknowledged()
    {
        var (workspaceId, _) = await SeedAsync();
        var client = CreateHttpClient();

        (await client.SendAsync(CreateRequest(HttpMethod.Put, "/api/app/ai-assist/settings", Manager, workspaceId, Settings())))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        (await client.SendAsync(CreateRequest(HttpMethod.Put, "/api/app/ai-assist/settings", Member, workspaceId, Settings())))
            .StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        using (var refused = await ReadAsync(await client.SendAsync(
                   CreateRequest(HttpMethod.Put, "/api/app/ai-assist/settings", Owner, workspaceId, Settings(ack: null))), HttpStatusCode.BadRequest))
        {
            var error = refused.RootElement.GetProperty("error");
            error.GetProperty("code").GetString().ShouldBe(TrackerDomainErrorCodes.AiDisclosureNotAccepted);
            error.GetProperty("data").GetProperty("version").GetString().ShouldBe("1.0");
            error.GetProperty("message").GetString().ShouldBe("Review and accept the AI disclosure (version 1.0) to turn on AI assist.");
        }

        using (var status = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Owner, workspaceId)), HttpStatusCode.OK))
        {
            status.RootElement.GetProperty("workspaceEnabled").GetBoolean().ShouldBeFalse();
        }
    }

    [Fact]
    public async Task A_Foreign_Workspace_Should_Be_Refused_Before_Any_AI_Code_Runs()
    {
        var (workspaceId, otherId) = await SeedAsync();
        var client = CreateHttpClient();

        foreach (var (method, route, body) in new (HttpMethod, string, object?)[]
                 {
                     (HttpMethod.Get, "/api/app/ai-assist/status", null),
                     (HttpMethod.Get, "/api/app/ai-assist/settings", null),
                     (HttpMethod.Put, "/api/app/ai-assist/settings", Settings()),
                     (HttpMethod.Get, "/api/app/ai-assist/usage", null)
                 })
        {
            var refused = await client.SendAsync(CreateRequest(method, route, Outsider, workspaceId, body));
            refused.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
            (await refused.Content.ReadAsStringAsync()).ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

            // Without a workspace at all.
            (await client.SendAsync(CreateRequest(method, route, Owner, null, body))).StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        }

        // The outsider's own workspace was not changed either.
        using var own = await ReadAsync(await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/ai-assist/status", Outsider, otherId)), HttpStatusCode.OK);
        own.RootElement.GetProperty("workspaceEnabled").GetBoolean().ShouldBeFalse();
        _provider.RequestedFeatures.ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Platform_Endpoints_Should_Be_Closed_To_Workspace_Users()
    {
        var (workspaceId, _) = await SeedAsync();
        var client = CreateHttpClient();

        foreach (var workspace in new Guid?[] { null, workspaceId })
        {
            (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/platform-ai/overview", Owner, workspace)))
                .StatusCode.ShouldBe(HttpStatusCode.Forbidden);
            (await client.SendAsync(CreateRequest(HttpMethod.Put, "/api/app/platform-ai/status", Owner, workspace, new { enabled = false })))
                .StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        }

        (await client.SendAsync(CreateRequest(HttpMethod.Get, "/api/app/platform-ai/overview", null, null)))
            .StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        (await WithUnitOfWorkAsync(() => GetRequiredService<AiSettingsManager>().IsPlatformEnabledAsync())).ShouldBeTrue();
    }
}
