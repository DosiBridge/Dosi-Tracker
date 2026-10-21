using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Projects;

/// <summary>
/// Project task lifecycle backend slice over hosted HTTP.
/// Exercises genuine TestServer HTTP endpoints (api/app/project-task), conventional MVC routing,
/// WorkspaceResolutionMiddleware, restored aggregate ABP permissions, project-level role scoping,
/// archive fail-closed guards, bounded input validation, and SQLite persistence isolation.
/// </summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspaceProjectTaskHttpTests : WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly Guid OwnerA = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid ColleagueA = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid DualUser = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid ForeignUserA = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid WithdrawnUserA = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb05");

    private static readonly Guid OwnerB = Guid.Parse("8a801e62-0953-4dd3-910b-dc6cc93ccb06");

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<ProjectTask, Guid> _taskRepository;
    private readonly IdentityUserManager _userManager;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceProjectTaskHttpTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _projectMemberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _taskRepository = GetRequiredService<IRepository<ProjectTask, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
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

    protected static new HttpRequestMessage CreateRequest(
        HttpMethod method,
        string url,
        Guid? userId,
        Guid? workspaceId,
        object? jsonBody = null)
    {
        var req = WorkspaceHttpTestBase<TrackerHttpApiServerTestModule>.CreateRequest(method, url, userId, workspaceId, jsonBody);
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        return req;
    }

    private async Task<(Guid WorkspaceA, Guid WorkspaceB, Guid ProjectA1, Guid ProjectA2, Guid ProjectA_Archived, Guid ProjectB1, Guid TaskB1Id)> SeedFixturesAsync()
    {
        var wsA = _guidGenerator.Create();
        var wsB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            if (await _userManager.FindByIdAsync(OwnerA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(OwnerA, "owner_a_tasks", "owner_a_tasks@acme.test") { Name = "Owner", Surname = "Alpha" });
            }
            if (await _userManager.FindByIdAsync(ColleagueA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ColleagueA, "colleague_a_tasks", "colleague_a_tasks@acme.test") { Name = "Colleague", Surname = "Alpha" });
            }
            if (await _userManager.FindByIdAsync(DualUser.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(DualUser, "dual_user_tasks", "dual_user_tasks@acme.test") { Name = "Dual", Surname = "User" });
            }
            if (await _userManager.FindByIdAsync(ForeignUserA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(ForeignUserA, "foreign_a_tasks", "foreign_a_tasks@acme.test") { Name = "Foreign", Surname = "Alpha" });
            }
            if (await _userManager.FindByIdAsync(WithdrawnUserA.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(WithdrawnUserA, "withdrawn_a_tasks", "withdrawn_a_tasks@acme.test") { Name = "Withdrawn", Surname = "Alpha" });
            }

            if (await _userManager.FindByIdAsync(OwnerB.ToString()) == null)
            {
                await _userManager.CreateAsync(new IdentityUser(OwnerB, "owner_b_tasks", "owner_b_tasks@acme.test") { Name = "Owner", Surname = "Beta" });
            }

            await _workspaceRepository.InsertAsync(new Workspace(wsA, "Workspace Tasks Alpha"), autoSave: true);
            await _workspaceRepository.InsertAsync(new Workspace(wsB, "Workspace Tasks Beta"), autoSave: true);

            // Workspace A members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, OwnerA, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ColleagueA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, ForeignUserA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsA, WithdrawnUserA, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);

            // Workspace B members
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, OwnerB, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true);
            await _memberRepository.InsertAsync(new WorkspaceMember(_guidGenerator.Create(), wsB, DualUser, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true);
        });

        Guid projA1 = Guid.Empty;
        Guid projA2 = Guid.Empty;
        Guid projA_Archived = Guid.Empty;
        Guid projB1 = Guid.Empty;
        Guid taskB1Id = Guid.Empty;

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                // Project A1: active, enrolled OwnerA (Admin), DualUser (Worker - contributor), WithdrawnUserA (Worker)
                var pA1 = new Project(_guidGenerator.Create(), wsA, "Project A1", color: "#112233");
                await _projectRepository.InsertAsync(pA1, autoSave: true);
                projA1 = pA1.Id;

                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA1, OwnerA, role: ProjectMemberRoles.Admin), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA1, DualUser, role: ProjectMemberRoles.Worker), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA1, WithdrawnUserA, role: ProjectMemberRoles.Worker), autoSave: true);

                // Project A2: active, enrolled ONLY OwnerA. ColleagueA is an active workspace member but NOT enrolled in Project A2
                var pA2 = new Project(_guidGenerator.Create(), wsA, "Project A2 Exclusive", color: "#223344");
                await _projectRepository.InsertAsync(pA2, autoSave: true);
                projA2 = pA2.Id;
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA2, OwnerA, role: ProjectMemberRoles.Admin), autoSave: true);

                // Project A_Archived: archived project
                var pAArch = new Project(_guidGenerator.Create(), wsA, "Project A Archived", color: "#334455");
                pAArch.Archive(DateTime.UtcNow.AddDays(-1));
                await _projectRepository.InsertAsync(pAArch, autoSave: true);
                projA_Archived = pAArch.Id;
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA_Archived, OwnerA, role: ProjectMemberRoles.Admin), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsA, projA_Archived, DualUser, role: ProjectMemberRoles.Worker), autoSave: true);

                // Seed existing task in Project A_Archived
                var archTask = new ProjectTask(_guidGenerator.Create(), wsA, projA_Archived, "Archived Preexisting Task", "Description");
                await _taskRepository.InsertAsync(archTask, autoSave: true);
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                // Project B1: active, enrolled OwnerB (Admin), DualUser (Client - read-only!)
                var pB1 = new Project(_guidGenerator.Create(), wsB, "Project B1", color: "#556677");
                await _projectRepository.InsertAsync(pB1, autoSave: true);
                projB1 = pB1.Id;

                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB1, OwnerB, role: ProjectMemberRoles.Admin), autoSave: true);
                await _projectMemberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), wsB, projB1, DualUser, role: ProjectMemberRoles.Client), autoSave: true);

                // Seed existing task in Project B1
                var tB1 = new ProjectTask(_guidGenerator.Create(), wsB, projB1, "Project B Seeded Task", "B sentinel");
                await _taskRepository.InsertAsync(tB1, autoSave: true);
                taskB1Id = tB1.Id;
            }
        });

        return (wsA, wsB, projA1, projA2, projA_Archived, projB1, taskB1Id);
    }

    [Fact]
    public async Task Authorized_Owner_And_Contributor_Can_Create_List_And_Update_Tasks_Over_Hosted_Http()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA creates task in Project A1
        var createOwnerReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "Task Created By Owner",
            Description = "Owner task description",
            IsCompleted = false
        });
        var createOwnerResp = await client.SendAsync(createOwnerReq);
        createOwnerResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var ownerTaskDto = JsonSerializer.Deserialize<ProjectTaskDto>(await createOwnerResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        ownerTaskDto.ShouldNotBeNull();
        ownerTaskDto.Title.ShouldBe("Task Created By Owner");
        ownerTaskDto.ProjectId.ShouldBe(projA1);
        ownerTaskDto.IsCompleted.ShouldBeFalse();
        ownerTaskDto.CompletedAt.ShouldBeNull();

        // 2. Contributor (DualUser with Worker role) creates task in Project A1
        var createContribReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "Task Created By Worker",
            Description = "Worker task description",
            IsCompleted = false
        });
        var createContribResp = await client.SendAsync(createContribReq);
        createContribResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var workerTaskDto = JsonSerializer.Deserialize<ProjectTaskDto>(await createContribResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        workerTaskDto.ShouldNotBeNull();
        workerTaskDto.Title.ShouldBe("Task Created By Worker");

        // 3. Contributor lists tasks in Project A1
        var listReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}", userId: DualUser, workspaceId: wsA);
        var listResp = await client.SendAsync(listReq);
        listResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var listResult = JsonSerializer.Deserialize<PagedResultDto<ProjectTaskDto>>(await listResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        listResult.ShouldNotBeNull();
        listResult.TotalCount.ShouldBe(2);
        listResult.Items.Count.ShouldBe(2);
        listResult.Items.ShouldContain(t => t.Id == ownerTaskDto.Id);
        listResult.Items.ShouldContain(t => t.Id == workerTaskDto.Id);

        // 4. Contributor updates task title and marks it completed
        var updateReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{workerTaskDto.Id}", userId: DualUser, workspaceId: wsA, new UpdateProjectTaskDto
        {
            Title = "Task Updated And Completed By Worker",
            Description = "Updated worker description",
            IsCompleted = true
        });
        var updateResp = await client.SendAsync(updateReq);
        updateResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var updatedDto = JsonSerializer.Deserialize<ProjectTaskDto>(await updateResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        updatedDto.ShouldNotBeNull();
        updatedDto.Title.ShouldBe("Task Updated And Completed By Worker");
        updatedDto.IsCompleted.ShouldBeTrue();
        updatedDto.CompletedAt.ShouldNotBeNull();

        // 5. Verify persisted directly in SQLite database
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var persisted = await _taskRepository.GetAsync(workerTaskDto.Id);
                persisted.Title.ShouldBe("Task Updated And Completed By Worker");
                persisted.IsCompleted.ShouldBeTrue();
                persisted.CompletedAt.ShouldNotBeNull();
            }
        });
    }

    [Fact]
    public async Task Contributor_Can_Toggle_Completion_State_Between_Completed_And_Reopened()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Create task as completed
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "Initial Completed Task",
            IsCompleted = true
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var taskDto = JsonSerializer.Deserialize<ProjectTaskDto>(await createResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        taskDto.ShouldNotBeNull();
        taskDto.IsCompleted.ShouldBeTrue();
        taskDto.CompletedAt.ShouldNotBeNull();

        // 2. Reopen the task (IsCompleted: false)
        var reopenReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{taskDto.Id}", userId: DualUser, workspaceId: wsA, new UpdateProjectTaskDto
        {
            Title = "Reopened Task",
            IsCompleted = false
        });
        var reopenResp = await client.SendAsync(reopenReq);
        reopenResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var reopenedDto = JsonSerializer.Deserialize<ProjectTaskDto>(await reopenResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        reopenedDto.ShouldNotBeNull();
        reopenedDto.IsCompleted.ShouldBeFalse();
        reopenedDto.CompletedAt.ShouldBeNull();

        // 3. Mark completed again (IsCompleted: true)
        var completeReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{taskDto.Id}", userId: DualUser, workspaceId: wsA, new UpdateProjectTaskDto
        {
            Title = "Re-completed Task",
            IsCompleted = true
        });
        var completeResp = await client.SendAsync(completeReq);
        completeResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var reCompletedDto = JsonSerializer.Deserialize<ProjectTaskDto>(await completeResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        reCompletedDto.ShouldNotBeNull();
        reCompletedDto.IsCompleted.ShouldBeTrue();
        reCompletedDto.CompletedAt.ShouldNotBeNull();
    }

    [Fact]
    public async Task Same_Account_Privileged_In_A_Cannot_Mutate_In_B_Where_Client_Role()
    {
        var (wsA, wsB, projA1, _, _, projB1, taskB1Id) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. In Workspace A, DualUser is Worker (contributor) -> can create task
        var createAReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "Task in A by DualUser"
        });
        var createAResp = await client.SendAsync(createAReq);
        createAResp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 2. DualUser switches to Workspace B where they hold ProjectMemberRoles.Client (read-only)
        // Can read tasks of Project B1
        var listBReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projB1}", userId: DualUser, workspaceId: wsB);
        var listBResp = await client.SendAsync(listBReq);
        listBResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var listBDto = JsonSerializer.Deserialize<PagedResultDto<ProjectTaskDto>>(await listBResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        listBDto.ShouldNotBeNull();
        listBDto.TotalCount.ShouldBe(1);
        listBDto.Items.First().Title.ShouldBe("Project B Seeded Task");

        // 3. DualUser attempts to create task in Project B1 -> rejected with 403 Forbidden (Tracker:RoleNotAllowed)
        var createBReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsB, new CreateProjectTaskDto
        {
            ProjectId = projB1,
            Title = "Unauthorized Task in B"
        });
        var createBResp = await client.SendAsync(createBReq);
        createBResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var createBBody = await createBResp.Content.ReadAsStringAsync();
        createBBody.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // 4. DualUser attempts to update seeded task in Project B1 -> rejected with 403 Forbidden (Tracker:RoleNotAllowed)
        var updateBReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{taskB1Id}", userId: DualUser, workspaceId: wsB, new UpdateProjectTaskDto
        {
            Title = "Mutated B Task Attempt",
            IsCompleted = true
        });
        var updateBResp = await client.SendAsync(updateBReq);
        updateBResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var updateBBody = await updateBResp.Content.ReadAsStringAsync();
        updateBBody.ShouldContain(TrackerDomainErrorCodes.RoleNotAllowed);

        // 5. DualUser switches back to Workspace A -> can mutate task in A1 again
        var createA2Req = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "Another Task in A by DualUser"
        });
        var createA2Resp = await client.SendAsync(createA2Req);
        createA2Resp.StatusCode.ShouldBe(HttpStatusCode.OK);

        // 6. Verify Workspace B task was NOT mutated in SQLite
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var taskB = await _taskRepository.GetAsync(taskB1Id);
                taskB.Title.ShouldBe("Project B Seeded Task");
                taskB.IsCompleted.ShouldBeFalse();
            }
        });
    }

    [Fact]
    public async Task Member_Not_Authorized_For_Target_Project_Is_Denied_On_Mutations_And_Reads()
    {
        var (wsA, _, _, projA2, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ColleagueA is active workspace member of wsA, but not enrolled in Project A2
        // 1. Create mutation rejected with 403 Forbidden (Tracker:NotProjectMember)
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: ColleagueA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA2,
            Title = "Unauthorized Project Task"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var createBody = await createResp.Content.ReadAsStringAsync();
        createBody.ShouldContain(TrackerDomainErrorCodes.NotProjectMember);

        // 2. Read query rejected with 404 Not Found (hiding project existence from non-members)
        var listReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA2}", userId: ColleagueA, workspaceId: wsA);
        var listResp = await client.SendAsync(listReq);
        listResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);
    }

    [Fact]
    public async Task Foreign_Workspace_Caller_Spoofing_Target_Header_Is_Denied_With_403_NotWorkspaceMember()
    {
        var (_, wsB, _, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // ForeignUserA belongs strictly to Workspace A, attempts task creation in Workspace B with spoofed header
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: ForeignUserA, workspaceId: wsB, new CreateProjectTaskDto
        {
            ProjectId = projB1,
            Title = "Spoofed Workspace Task"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await createResp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);
    }

    [Fact]
    public async Task Authorized_Workspace_A_Mutation_Using_Real_Workspace_B_Project_Or_Task_Returns_404()
    {
        var (wsA, wsB, _, _, _, projB1, taskB1Id) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. OwnerA in Workspace A creates task targeting real Project B1 ID from Workspace B
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projB1,
            Title = "Cross-tenant task attempt"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // 2. OwnerA in Workspace A updates real Task B1 ID from Workspace B
        var updateReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{taskB1Id}", userId: OwnerA, workspaceId: wsA, new UpdateProjectTaskDto
        {
            Title = "Cross-tenant update attempt",
            IsCompleted = true
        });
        var updateResp = await client.SendAsync(updateReq);
        updateResp.StatusCode.ShouldBe(HttpStatusCode.NotFound);

        // Verify Workspace B task remained completely unaltered
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var taskB = await _taskRepository.GetAsync(taskB1Id);
                taskB.Title.ShouldBe("Project B Seeded Task");
                taskB.IsCompleted.ShouldBeFalse();
            }
        });
    }

    [Fact]
    public async Task Archived_Project_Rejects_Mutations_With_409_Conflict_ProjectArchived_While_Permitting_Read()
    {
        var (wsA, _, _, _, projA_Archived, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Create task in archived project is rejected with 409 Conflict (Tracker:ProjectArchived)
        var createReq = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA_Archived,
            Title = "New Task in Archived Project"
        });
        var createResp = await client.SendAsync(createReq);
        createResp.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var createBody = await createResp.Content.ReadAsStringAsync();
        createBody.ShouldContain(TrackerDomainErrorCodes.ProjectArchived);

        // Find existing task in archived project
        Guid archTaskId = Guid.Empty;
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var t = await _taskRepository.FirstOrDefaultAsync(t => t.ProjectId == projA_Archived);
                archTaskId = t!.Id;
            }
        });

        // 2. Update task in archived project is rejected with 409 Conflict (Tracker:ProjectArchived)
        var updateReq = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{archTaskId}", userId: OwnerA, workspaceId: wsA, new UpdateProjectTaskDto
        {
            Title = "Mutate Archived Task",
            IsCompleted = true
        });
        var updateResp = await client.SendAsync(updateReq);
        updateResp.StatusCode.ShouldBe(HttpStatusCode.Conflict);
        var updateBody = await updateResp.Content.ReadAsStringAsync();
        updateBody.ShouldContain(TrackerDomainErrorCodes.ProjectArchived);

        // 3. Reading existing tasks in archived project is PERMITTED (200 OK)
        var listReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA_Archived}", userId: OwnerA, workspaceId: wsA);
        var listResp = await client.SendAsync(listReq);
        listResp.StatusCode.ShouldBe(HttpStatusCode.OK);
        var listResult = JsonSerializer.Deserialize<PagedResultDto<ProjectTaskDto>>(await listResp.Content.ReadAsStringAsync(), CamelCaseJsonOptions);
        listResult.ShouldNotBeNull();
        listResult.TotalCount.ShouldBe(1);
        listResult.Items.First().Title.ShouldBe("Archived Preexisting Task");
    }

    [Fact]
    public async Task Archived_Workspace_Rejects_Task_Operations_With_403_Forbidden_WorkspaceArchived()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Archive Workspace A
        await WithUnitOfWorkAsync(async () =>
        {
            var ws = await _workspaceRepository.GetAsync(wsA);
            ws.Archive(OwnerA);
            await _workspaceRepository.UpdateAsync(ws, autoSave: true);
        });

        // Any task operation targeting archived workspace is rejected by WorkspaceResolutionMiddleware with 403 Forbidden (Tracker:WorkspaceArchived)
        var listReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}", userId: OwnerA, workspaceId: wsA);
        var listResp = await client.SendAsync(listReq);
        listResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var body = await listResp.Content.ReadAsStringAsync();
        body.ShouldContain(TrackerDomainErrorCodes.WorkspaceArchived);
    }

    [Fact]
    public async Task Missing_Or_Empty_Workspace_Header_Is_Rejected_With_400_BadRequest()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Missing workspace header
        var missingReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}", userId: OwnerA, workspaceId: null);
        var missingResp = await client.SendAsync(missingReq);
        missingResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var missingBody = await missingResp.Content.ReadAsStringAsync();
        missingBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);

        // 2. Guid.Empty workspace header
        var emptyReq = new HttpRequestMessage(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}");
        emptyReq.Headers.Authorization = new AuthenticationHeaderValue("Bearer", OwnerA.ToString());
        emptyReq.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        emptyReq.Headers.Add(WorkspaceConsts.HeaderName, Guid.Empty.ToString());

        var emptyResp = await client.SendAsync(emptyReq);
        emptyResp.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
        var emptyBody = await emptyResp.Content.ReadAsStringAsync();
        emptyBody.ShouldContain(TrackerDomainErrorCodes.WorkspaceRequired);
    }

    [Fact]
    public async Task Unauthenticated_Request_Is_Rejected_With_401_Unauthorized()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        var req = new HttpRequestMessage(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}");
        req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/json"));
        req.Headers.Add(WorkspaceConsts.HeaderName, wsA.ToString());

        var resp = await client.SendAsync(req);
        resp.StatusCode.ShouldBe(HttpStatusCode.Unauthorized);
        var body = await resp.Content.ReadAsStringAsync();
        body.ShouldContain("010001");
    }

    [Fact]
    public async Task Withdrawn_Member_From_Workspace_Is_Denied_On_Fresh_Task_Request()
    {
        var (wsA, wsB, projA1, _, _, projB1, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. WithdrawnUserA departs via production self-service leave endpoint
        var leaveReq = CreateRequest(HttpMethod.Post, $"/api/app/workspaces/{wsA}/leave", userId: WithdrawnUserA, workspaceId: null);
        var leaveResp = await client.SendAsync(leaveReq);
        (leaveResp.StatusCode == HttpStatusCode.OK || leaveResp.StatusCode == HttpStatusCode.NoContent).ShouldBeTrue();

        // 2. Fresh task request from withdrawn user is rejected with 403 Forbidden (Tracker:NotWorkspaceMember)
        var freshReq = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projA1}", userId: WithdrawnUserA, workspaceId: wsA);
        var freshResp = await client.SendAsync(freshReq);
        freshResp.StatusCode.ShouldBe(HttpStatusCode.Forbidden);
        var freshBody = await freshResp.Content.ReadAsStringAsync();
        freshBody.ShouldContain(TrackerDomainErrorCodes.NotWorkspaceMember);

        // 3. Unrelated Workspace B remains operational
        var reqB = CreateRequest(HttpMethod.Get, $"/api/app/project-task?projectId={projB1}", userId: OwnerB, workspaceId: wsB);
        var respB = await client.SendAsync(reqB);
        respB.StatusCode.ShouldBe(HttpStatusCode.OK);
    }

    [Fact]
    public async Task Target_Workspace_Persisted_Sentinels_Remain_Unchanged_Across_Battery_Of_Denied_Requests()
    {
        var (wsA, wsB, projA1, projA2, _, projB1, taskB1Id) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // Denied calls:
        // 1. ColleagueA (not in projA2) tries to create task in projA2
        var req1 = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: ColleagueA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA2,
            Title = "Denied 1"
        });
        await client.SendAsync(req1);

        // 2. DualUser (Client in projB1) tries to create task in projB1
        var req2 = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: DualUser, workspaceId: wsB, new CreateProjectTaskDto
        {
            ProjectId = projB1,
            Title = "Denied 2"
        });
        await client.SendAsync(req2);

        // 3. ForeignUserA tries to mutate task in wsB
        var req3 = CreateRequest(HttpMethod.Put, $"/api/app/project-task/{taskB1Id}", userId: ForeignUserA, workspaceId: wsB, new UpdateProjectTaskDto
        {
            Title = "Denied 3",
            IsCompleted = true
        });
        await client.SendAsync(req3);

        // Verify sentinels
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentWorkspace.Change(wsA, WorkspaceRoles.Owner))
            {
                var tasksA2 = await _taskRepository.GetListAsync(t => t.ProjectId == projA2);
                tasksA2.ShouldBeEmpty();

                var proj1 = await _projectRepository.GetAsync(projA1);
                proj1.Title.ShouldBe("Project A1");
            }

            using (_currentWorkspace.Change(wsB, WorkspaceRoles.Owner))
            {
                var tasksB1 = await _taskRepository.GetListAsync(t => t.ProjectId == projB1);
                tasksB1.Count.ShouldBe(1);
                tasksB1.Single().Title.ShouldBe("Project B Seeded Task");
                tasksB1.Single().IsCompleted.ShouldBeFalse();
            }
        });
    }

    [Fact]
    public async Task Bounded_Input_Validation_Rejects_Invalid_Titles_With_400_BadRequest()
    {
        var (wsA, _, projA1, _, _, _, _) = await SeedFixturesAsync();
        var client = CreateHttpClient();

        // 1. Empty title
        var reqEmpty = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = ""
        });
        var respEmpty = await client.SendAsync(reqEmpty);
        respEmpty.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 2. Whitespace-only title
        var reqWs = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = "   "
        });
        var respWs = await client.SendAsync(reqWs);
        respWs.StatusCode.ShouldBe(HttpStatusCode.BadRequest);

        // 3. Title exceeding 256 characters
        var reqLong = CreateRequest(HttpMethod.Post, "/api/app/project-task", userId: OwnerA, workspaceId: wsA, new CreateProjectTaskDto
        {
            ProjectId = projA1,
            Title = new string('x', 257)
        });
        var respLong = await client.SendAsync(reqLong);
        respLong.StatusCode.ShouldBe(HttpStatusCode.BadRequest);
    }
}
