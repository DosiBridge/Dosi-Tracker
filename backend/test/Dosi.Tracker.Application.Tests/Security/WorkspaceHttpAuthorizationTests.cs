using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Claims;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using System.Diagnostics;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Authorization.Policy;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Microsoft.Extensions.Options;
using Microsoft.Extensions.Hosting;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Volo.Abp.AspNetCore.Mvc;
using Volo.Abp.Autofac;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.TestHost;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// REQ-ISO-05: Authenticated HTTP aggregate authorization and workspace isolation integration harness.
/// Sends real in-process HTTP requests through production workspace resolution, SQLite membership lookup,
/// actual ABP aggregate permission checking (WorkspaceRolePermissionValueProvider + IPermissionChecker),
/// and real production ProjectAppService / EF Core query filter paths.
/// Uses a test-only authentication handler for synthetic identities without copied policies or fake endpoints.
/// </summary>
public abstract class WorkspaceHttpAuthorizationTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid UserA = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid UserB = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid DualUser = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid InactiveUser = Guid.Parse("3f701e62-0953-4dd3-910b-dc6cc93ccb04");

    private static readonly JsonSerializerOptions CamelCaseJsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    private static Guid ReadGuidProperty(JsonElement element, string propName)
    {
        if (element.TryGetProperty(propName.ToLowerInvariant(), out var p) ||
            element.TryGetProperty(char.ToUpperInvariant(propName[0]) + propName[1..], out p))
        {
            return Guid.Parse(p.GetString()!);
        }
        throw new KeyNotFoundException($"Property '{propName}' not found in JSON payload");
    }

    private static string ReadStringProperty(JsonElement element, string propName)
    {
        if (element.TryGetProperty(propName.ToLowerInvariant(), out var p) ||
            element.TryGetProperty(char.ToUpperInvariant(propName[0]) + propName[1..], out p))
        {
            return p.GetString()!;
        }
        throw new KeyNotFoundException($"Property '{propName}' not found in JSON payload");
    }

    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    protected WorkspaceHttpAuthorizationTests()
    {
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);

        var diagnosticListener = new DiagnosticListener("Microsoft.AspNetCore");
        services.AddSingleton(diagnosticListener);
        services.AddSingleton<DiagnosticSource>(diagnosticListener);

        services.AddRouting();
        services.AddAuthorization();
        services.AddAuthorizationPolicyEvaluator();

        // Restore real production ABP aggregate authorization service and permission checker
        // (undoing TrackerTestBaseModule's AddAlwaysAllowAuthorization)
        services.Replace(ServiceDescriptor.Transient<IAuthorizationService, AbpAuthorizationService>());
        services.Replace(ServiceDescriptor.Transient<IAbpAuthorizationService, AbpAuthorizationService>());
        services.Replace(ServiceDescriptor.Transient<IPermissionChecker, PermissionChecker>());

        services.AddAuthentication(options =>
        {
            options.DefaultAuthenticateScheme = TestAuthHandler.SchemeName;
            options.DefaultChallengeScheme = TestAuthHandler.SchemeName;
        })
        .AddScheme<AuthenticationSchemeOptions, TestAuthHandler>(TestAuthHandler.SchemeName, _ => { });
    }

    public class TestAuthHandler : AuthenticationHandler<AuthenticationSchemeOptions>
    {
        public const string SchemeName = "TestAuth";

        public TestAuthHandler(
            IOptionsMonitor<AuthenticationSchemeOptions> options,
            Microsoft.Extensions.Logging.ILoggerFactory logger,
            System.Text.Encodings.Web.UrlEncoder encoder)
            : base(options, logger, encoder)
        {
        }

        protected override Task<AuthenticateResult> HandleAuthenticateAsync()
        {
            string? rawUserId = null;
            if (Request.Headers.TryGetValue("Authorization", out var authHeader))
            {
                var val = authHeader.ToString();
                if (val.StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase))
                {
                    rawUserId = val["Bearer ".Length..].Trim();
                }
                else if (val.StartsWith("Test ", StringComparison.OrdinalIgnoreCase))
                {
                    rawUserId = val["Test ".Length..].Trim();
                }
                else
                {
                    rawUserId = val.Trim();
                }
            }
            else if (Request.Headers.TryGetValue("X-Test-User-Id", out var userHeader))
            {
                rawUserId = userHeader.ToString().Trim();
            }

            if (string.IsNullOrWhiteSpace(rawUserId) || !Guid.TryParse(rawUserId, out var userId))
            {
                return Task.FromResult(AuthenticateResult.NoResult());
            }

            var identity = new ClaimsIdentity(new[]
            {
                new Claim(AbpClaimTypes.UserId, userId.ToString()),
                new Claim(ClaimTypes.NameIdentifier, userId.ToString())
            }, SchemeName);

            var principal = new ClaimsPrincipal(identity);
            var ticket = new AuthenticationTicket(principal, SchemeName);
            return Task.FromResult(AuthenticateResult.Success(ticket));
        }

        protected override Task HandleChallengeAsync(AuthenticationProperties properties)
        {
            Response.StatusCode = StatusCodes.Status401Unauthorized;
            Response.ContentType = "application/json";
            return Response.WriteAsync(JsonSerializer.Serialize(new
            {
                error = new
                {
                    code = "Volo.Abp.Authorization:010001",
                    message = "Current user did not login to the application!"
                }
            }));
        }

        protected override Task HandleForbiddenAsync(AuthenticationProperties properties)
        {
            Response.StatusCode = StatusCodes.Status403Forbidden;
            Response.ContentType = "application/json";
            return Response.WriteAsync(JsonSerializer.Serialize(new
            {
                error = new
                {
                    code = "Volo.Abp.Authorization:010002",
                    message = "Authorization failed! Given policy has not granted."
                }
            }));
        }
    }

    protected virtual void ConfigureTestServerServices(IServiceCollection services) { }

    private HttpClient CreateHttpClient()
    {
        var config = ServiceProvider.GetRequiredService<Microsoft.Extensions.Configuration.IConfiguration>();

        var host = new Microsoft.Extensions.Hosting.HostBuilder()
            .UseAutofac()
            .ConfigureWebHost(web => web
                .UseTestServer()
                .ConfigureServices(services =>
                {
                    services.ReplaceConfiguration(config);
                    services.AddLogging();
                    services.AddRouting();
                    services.AddAuthentication(options =>
                    {
                        options.DefaultAuthenticateScheme = TestAuthHandler.SchemeName;
                        options.DefaultChallengeScheme = TestAuthHandler.SchemeName;
                    })
                    .AddScheme<AuthenticationSchemeOptions, TestAuthHandler>(TestAuthHandler.SchemeName, _ => { });

                    services.Configure<Volo.Abp.BackgroundWorkers.AbpBackgroundWorkerOptions>(options => options.IsEnabled = false);

                    services.AddApplication<TStartupModule>();
                    ConfigureTestServerServices(services);

                    services.Replace(ServiceDescriptor.Singleton<ICurrentWorkspace, CurrentWorkspace>());
                    services.Replace(ServiceDescriptor.Transient<IAuthorizationService, AbpAuthorizationService>());
                    services.Replace(ServiceDescriptor.Transient<IAbpAuthorizationService, AbpAuthorizationService>());
                    services.Replace(ServiceDescriptor.Transient<IPermissionChecker, PermissionChecker>());
                })
                .Configure(app =>
                {
                    app.InitializeApplication();
                    app.UseRouting();
                    app.UseAuthentication();

                    app.Use(async (context, next) =>
                    {
                        var principalAccessor = context.RequestServices.GetRequiredService<ICurrentPrincipalAccessor>();
                        using (principalAccessor.Change(context.User))
                        {
                            await next();
                        }
                    });

                    app.UseUnitOfWork();
                    app.UseWorkspaceResolution();
                    app.UseAuthorization();

                    app.UseEndpoints(endpoints =>
                    {
                        endpoints.MapControllers();
                    });
                }))
            .Build();

        host.Start();
        return host.GetTestServer().CreateClient();
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB)> SeedUsersAndWorkspacesAsync()
    {
        if (await _userManager.FindByIdAsync(UserA.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserA, "userA_http", "userA_http@acme.test"));
        }
        if (await _userManager.FindByIdAsync(UserB.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(UserB, "userB_http", "userB_http@acme.test"));
        }
        if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(DualUser, "dualUser_http", "dualUser_http@acme.test"));
        }
        if (await _userManager.FindByIdAsync(InactiveUser.ToString()) == null)
        {
            await _userManager.CreateAsync(new IdentityUser(InactiveUser, "inactiveUser_http", "inactiveUser_http@acme.test"));
        }

        var wsA = await ServiceProvider.CreateWorkspaceAsync("HTTP-WS-A");
        var wsB = await ServiceProvider.CreateWorkspaceAsync("HTTP-WS-B");

        // UserA: Manager in Workspace A only
        await ServiceProvider.AddWorkspaceMemberAsync(UserA, WorkspaceRoles.Manager, wsA);

        // UserB: Owner in Workspace B only
        await ServiceProvider.AddWorkspaceMemberAsync(UserB, WorkspaceRoles.Owner, wsB);

        // DualUser: Manager in Workspace A, Member in Workspace B
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Manager, wsA);
        await ServiceProvider.AddWorkspaceMemberAsync(DualUser, WorkspaceRoles.Member, wsB);

        // InactiveUser: Removed (Inactive) in Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var member = new WorkspaceMember(_guidGenerator.Create(), wsA, InactiveUser, WorkspaceRoles.Member, DateTime.UtcNow);
            member.Remove();
            await _memberRepository.InsertAsync(member, autoSave: true);
        });

        return (wsA, wsB);
    }

    private static HttpRequestMessage CreateRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        Guid? workspaceId,
        object? jsonBody = null)
    {
        var request = new HttpRequestMessage(method, url);
        if (userId.HasValue)
        {
            request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", userId.Value.ToString());
        }

        if (workspaceId.HasValue)
        {
            request.Headers.Add(WorkspaceConsts.HeaderName, workspaceId.Value.ToString());
        }

        if (jsonBody != null)
        {
            var json = JsonSerializer.Serialize(jsonBody);
            request.Content = new StringContent(json, Encoding.UTF8, "application/json");
        }

        return request;
    }

    [Fact]
    public async Task Unauthenticated_Request_To_Protected_Project_Endpoint_Is_Rejected_With_401()
    {
        var (wsA, _) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        var request = CreateRequest(HttpMethod.Get, "/api/app/project", userId: null, workspaceId: wsA);
        var response = await client.SendAsync(request);

        response.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
    }

    [Fact]
    public async Task Missing_Workspace_Header_Is_Rejected_With_400_WorkspaceRequired_By_Action_Filter()
    {
        await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        // Production missing header behavior: passes WorkspaceResolutionMiddleware,
        // intercepted by production WorkspaceRequiredFilter on conventional controller,
        // which throws BusinessException(Tracker:WorkspaceRequired) -> ABP maps to 400 BadRequest
        var missingHeaderReq = new HttpRequestMessage(HttpMethod.Get, "/api/app/project");
        missingHeaderReq.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());

        var missingResponse = await client.SendAsync(missingHeaderReq);
        missingResponse.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        var missingBody = await missingResponse.Content.ReadAsStringAsync();
        missingBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Empty_Workspace_Header_Guid_Empty_Is_Rejected_With_400_WorkspaceRequired_By_Resolution_Middleware()
    {
        await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        // Guid.Empty rejected with 400 Tracker:WorkspaceRequired directly by WorkspaceResolutionMiddleware
        var emptyHeaderReq = new HttpRequestMessage(HttpMethod.Get, "/api/app/project");
        emptyHeaderReq.Headers.Authorization = new AuthenticationHeaderValue("Bearer", UserA.ToString());
        emptyHeaderReq.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());

        var emptyResponse = await client.SendAsync(emptyHeaderReq);
        emptyResponse.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        var emptyBody = await emptyResponse.Content.ReadAsStringAsync();
        emptyBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Workspace_A_Only_Caller_Spoofing_Workspace_B_Header_Is_Rejected_With_403_NotWorkspaceMember()
    {
        var (wsA, wsB) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        // Seed target workspace B sentinel to ensure non-vacuous inspection
        Guid sentinelBId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var sentinel = new Project(_guidGenerator.Create(), wsB, "Sentinel-B-Project", color: "#990000");
                await _projectRepository.InsertAsync(sentinel, autoSave: true);
                sentinelBId = sentinel.Id;
            }
        });

        // 1. UserA attempts GET on Workspace B
        var getReq = CreateRequest(HttpMethod.Get, "/api/app/project", userId: UserA, workspaceId: wsB);
        var getResponse = await client.SendAsync(getReq);

        getResponse.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var getBody = await getResponse.Content.ReadAsStringAsync();
        getBody.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
        getBody.ShouldContain("You are not a member of this workspace.");

        // 2. UserA attempts mutation (POST) on Workspace B
        var postReq = CreateRequest(HttpMethod.Post, "/api/app/project", userId: UserA, workspaceId: wsB, new
        {
            Title = "Spoofed Project in B",
            Color = "#112233"
        });
        var postResponse = await client.SendAsync(postReq);

        postResponse.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var postBody = await postResponse.Content.ReadAsStringAsync();
        postBody.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // 3. Non-vacuous SQLite DB state inspection under explicit Workspace B context:
        // Confirms that target workspace B was not empty, pre-seeded sentinel remains intact,
        // and spoofed POST mutation did not insert any additional entity.
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var wsBProjects = await _projectRepository.GetListAsync(p => p.WorkspaceId == wsB);
                wsBProjects.Count.ShouldBe(1, "Target workspace B must retain only its pre-seeded sentinel");
                wsBProjects.ShouldContain(p => p.Id == sentinelBId && p.Title == "Sentinel-B-Project");
                wsBProjects.ShouldNotContain(p => p.Title == "Spoofed Project in B");
            }
        });
    }

    [Fact]
    public async Task Cross_Workspace_Resource_Id_Read_And_Mutation_While_Selecting_Workspace_A_Are_Rejected_And_Target_Unchanged()
    {
        var (wsA, wsB) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        Guid sentinelAId = Guid.Empty;
        Guid sentinelBId = Guid.Empty;

        // Seed distinct sentinels under Workspace A and Workspace B
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var pA = new Project(_guidGenerator.Create(), wsA, "Sentinel-A-Project", color: "#111111");
                await _projectRepository.InsertAsync(pA, autoSave: true);
                sentinelAId = pA.Id;
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var pB = new Project(_guidGenerator.Create(), wsB, "Sentinel-B-Project", color: "#222222");
                await _projectRepository.InsertAsync(pB, autoSave: true);
                sentinelBId = pB.Id;
            }
        });

        // 1. Cross-workspace resource-ID read: UserA selects Workspace A, requests Workspace B sentinel ID
        var crossGetReq = CreateRequest(HttpMethod.Get, $"/api/app/project/{sentinelBId}", userId: UserA, workspaceId: wsA);
        var crossGetResp = await client.SendAsync(crossGetReq);
        crossGetResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // 2. Cross-workspace resource-ID update: UserA selects Workspace A, attempts PUT mutation on Workspace B sentinel ID
        var crossPutReq = CreateRequest(HttpMethod.Put, $"/api/app/project/{sentinelBId}", userId: UserA, workspaceId: wsA, new
        {
            Title = "Tampered-By-A",
            Color = "#999999"
        });
        var crossPutResp = await client.SendAsync(crossPutReq);
        crossPutResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // 3. Cross-workspace resource-ID delete: UserA selects Workspace A, attempts DELETE on Workspace B sentinel ID.
        // UserA has role Manager in Workspace A; Manager does not have Projects.Delete permission in production,
        // so request is denied with 403 Forbidden per production contract.
        var crossDelReq = CreateRequest(HttpMethod.Delete, $"/api/app/project/{sentinelBId}", userId: UserA, workspaceId: wsA);
        var crossDelResp = await client.SendAsync(crossDelReq);
        crossDelResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);

        // 4. Verify persisted state:
        // Workspace A state and counts unchanged
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var wsAProjects = await _projectRepository.GetListAsync(p => p.WorkspaceId == wsA);
                wsAProjects.Count.ShouldBe(1);
                wsAProjects.ShouldContain(p => p.Id == sentinelAId && p.Title == "Sentinel-A-Project");
            }

            // Target Workspace B state and counts unchanged, sentinel entity content unaltered
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var wsBProjects = await _projectRepository.GetListAsync(p => p.WorkspaceId == wsB);
                wsBProjects.Count.ShouldBe(1);
                var bEntity = wsBProjects.Single(p => p.Id == sentinelBId);
                bEntity.Title.ShouldBe("Sentinel-B-Project");
                bEntity.Color.ShouldBe("#222222");
            }
        });
    }

    [Fact]
    public async Task Member_Role_Caller_Is_Denied_Manager_Operation_By_Actual_Aggregate_Authorization()
    {
        var (_, wsB) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        // DualUser is an active member in Workspace B, but holds role 'Member' (lacking Projects.Create)
        var postReq = CreateRequest(HttpMethod.Post, "/api/app/project", userId: DualUser, workspaceId: wsB, new
        {
            Title = "Unauthorized Member Project",
            Color = "#112233"
        });
        var postResponse = await client.SendAsync(postReq);

        postResponse.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var postBody = await postResponse.Content.ReadAsStringAsync();
        postBody.ShouldContain("Volo.Abp.Authorization:010002");

        // Verify SQLite DB state: project was NOT created in Workspace B
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var wsBProjects = await _projectRepository.GetListAsync(p => p.WorkspaceId == wsB);
                wsBProjects.ShouldBeEmpty("Denied member mutation must leave target workspace database state unchanged");
            }
        });
    }

    [Fact]
    public async Task Dual_Member_Sees_Only_Selected_Workspace_Data_And_Persisted_Roles_Govern_Operations()
    {
        var (wsA, wsB) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        // 1. DualUser creates "Project-A1" under Workspace A (where DualUser is Manager -> permitted)
        var createReqA = CreateRequest(HttpMethod.Post, "/api/app/project", userId: DualUser, workspaceId: wsA, new
        {
            Title = "Project-A1",
            Color = "#112233"
        });
        var createRespA = await client.SendAsync(createReqA);
        createRespA.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. UserB creates "Project-B1" under Workspace B (where UserB is Owner -> permitted)
        var createReqB = CreateRequest(HttpMethod.Post, "/api/app/project", userId: UserB, workspaceId: wsB, new
        {
            Title = "Project-B1",
            Color = "#445566"
        });
        var createRespB = await client.SendAsync(createReqB);
        createRespB.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 3. DualUser queries under Workspace A -> sees ONLY Project-A1
        var listReqA = CreateRequest(HttpMethod.Get, "/api/app/project", userId: DualUser, workspaceId: wsA);
        var listRespA = await client.SendAsync(listReqA);
        listRespA.StatusCode.ShouldBe(HttpStatusCode.OK);
        var listBodyA = await listRespA.Content.ReadAsStringAsync();
        listBodyA.ShouldContain("Project-A1");
        listBodyA.ShouldNotContain("Project-B1");

        // 4. DualUser queries under Workspace B -> sees ONLY Project-B1
        // Note: ProjectAppService.CreateFilteredQueryAsync filters projects by membership for non-managers.
        // First enroll DualUser in Project-B1 so they have read visibility
        Guid projectB1Id;
        using (var doc = JsonDocument.Parse(await createRespB.Content.ReadAsStringAsync()))
        {
            projectB1Id = ReadGuidProperty(doc.RootElement, "id");
        }

        await WithUnitOfWorkAsync(async () =>
        {
            var projectMemberRepo = GetRequiredService<IRepository<ProjectMember, Guid>>();
            await projectMemberRepo.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projectB1Id, DualUser), autoSave: true);
        });

        var listReqB = CreateRequest(HttpMethod.Get, "/api/app/project", userId: DualUser, workspaceId: wsB);
        var listRespB = await client.SendAsync(listReqB);
        listRespB.StatusCode.ShouldBe(HttpStatusCode.OK);
        var listBodyB = await listRespB.Content.ReadAsStringAsync();
        listBodyB.ShouldContain("Project-B1");
        listBodyB.ShouldNotContain("Project-A1");

        // 5. DualUser tries to create under Workspace B -> rejected because role is Member
        var createDeniedB = CreateRequest(HttpMethod.Post, "/api/app/project", userId: DualUser, workspaceId: wsB, new
        {
            Title = "Project-B-Denied",
            Color = "#778899"
        });
        var createDeniedRespB = await client.SendAsync(createDeniedB);
        createDeniedRespB.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
    }

    [Fact]
    public async Task Inactive_Or_Removed_Workspace_Member_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (wsA, _) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        var request = CreateRequest(HttpMethod.Get, "/api/app/project", userId: InactiveUser, workspaceId: wsA);
        var response = await client.SendAsync(request);

        response.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await response.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
    }

    [Fact]
    public async Task Legitimate_Permitted_Path_Creates_Project_In_Workspace_And_Persists_To_Database()
    {
        var (wsA, _) = await SeedUsersAndWorkspacesAsync();
        var client = CreateHttpClient();

        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project", userId: UserA, workspaceId: wsA, new
        {
            Title = "Permitted-Project-A",
            Color = "#22C55E"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        var respBody = await createResp.Content.ReadAsStringAsync();
        using var doc = JsonDocument.Parse(respBody);
        var createdId = ReadGuidProperty(doc.RootElement, "id");
        ReadStringProperty(doc.RootElement, "title").ShouldBe("Permitted-Project-A");
        ReadStringProperty(doc.RootElement, "color").ShouldBe("#22C55E");

        // Verify direct SQLite database persistence under Workspace A context
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var project = await _projectRepository.FindAsync(createdId);
                project.ShouldNotBeNull();
                project.WorkspaceId.ShouldBe(wsA);
                project.Title.ShouldBe("Permitted-Project-A");
                project.Color.ShouldBe("#22C55E");
            }

            // Cross-workspace query isolation verification: project is invisible under Workspace B context
            using (_currentWorkspace.Change(Guid.NewGuid(), WorkspaceRoles.Owner))
            {
                var foreignProject = await _projectRepository.FindAsync(createdId);
                foreignProject.ShouldBeNull("Workspace A project must not be returned under foreign workspace context");
            }
        });
    }
}
