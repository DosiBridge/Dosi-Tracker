using System;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Security;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Auditing;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>Task drafts: only for someone who could add the tasks, never created by AI, with no fallback.</summary>
public abstract class TaskDraftsTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private const string Goal = "Launch the new pricing page before the sales kickoff";

    private static string Answer(params object[] drafts) => Json(new { drafts });

    private static object Draft(string title, string description = "", string rationale = "It moves the launch forward.", string duplicateOf = "") =>
        new { title, description, rationale, duplicateOf };

    private Task<int> TaskCountAsync() =>
        WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<ProjectTask, Guid>>().GetListAsync()).Count);

    private Task<Guid> SeededTaskIdAsync() =>
        WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<ProjectTask, Guid>>().GetListAsync()).Single().Id);

    [Fact]
    public async Task Drafts_Should_Be_Returned_With_Duplicates_Flagged_And_Nothing_Created()
    {
        await ReadyAsync();
        var before = await TaskCountAsync();
        Model.EnqueueJson(Answer(
            Draft("Write the pricing copy", "Draft the copy for each plan of {P1}."),
            Draft("Ship the pricing page", rationale: "It repeats an open task.", duplicateOf: "T1")));

        var result = await Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal });

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Meta.DataCategories.ShouldBe(new[] { AiDataCategories.ProjectNames, AiDataCategories.TaskTitles });
        result.Drafts.Count.ShouldBe(2);
        result.Drafts[0].Title.ShouldBe("Write the pricing copy");
        result.Drafts[0].Description.ShouldBe("Draft the copy for each plan of Website.");
        result.Drafts[0].PossibleDuplicateOfTaskId.ShouldBeNull();
        result.Drafts[1].Description.ShouldBeNull();
        result.Drafts[1].PossibleDuplicateOfTaskId.ShouldBe(await SeededTaskIdAsync());
        (await TaskCountAsync()).ShouldBe(before);

        var call = Model.Calls.ShouldHaveSingleItem();
        ShouldHoldNothingPrivate(call);
        call.AllText.ShouldContain(Goal);
        call.AllText.ShouldContain("Ship the pricing page");
        (await RowsAsync()).ShouldHaveSingleItem().Audience.ShouldBe(AiUsageRecord.ProjectAudience);
    }

    [Fact]
    public async Task Drafts_Should_Be_Clamped_To_The_Request_And_The_Task_Limits()
    {
        await ReadyAsync();
        var longTitle = "Write " + new string('x', 400);
        Model.EnqueueJson(Answer(Draft(longTitle, new string('y', 900)), Draft("Second draft"), Draft("Third draft")));

        var result = await Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal, MaxTasks = 2 });

        result.Drafts.Count.ShouldBe(2);
        result.Drafts[0].Title.Length.ShouldBe(ProjectTaskConsts.MaxTitleLength);
        result.Drafts[0].Description!.Length.ShouldBe(TaskDraftsFeature.MaxDescriptionLength);
    }

    [Fact]
    public async Task Output_That_Stays_Invalid_Should_Be_An_Error_Not_A_Fallback()
    {
        await ReadyAsync();
        var bad = Answer(Draft("Ask {ME} to check the 7 plans"));
        Model.EnqueueJson(bad).EnqueueJson(bad);

        var invalid = await Should.ThrowAsync<BusinessException>(() =>
            Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal }));

        invalid.Code.ShouldBe(TrackerDomainErrorCodes.AiOutputInvalid);
        Model.Calls.Count.ShouldBe(2);
    }

    [Fact]
    public async Task A_Provider_Failure_Should_Be_An_Error_Not_A_Fallback()
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.RateLimited, 429, 7));

        var unavailable = await Should.ThrowAsync<BusinessException>(() =>
            Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal }));

        unavailable.Code.ShouldBe(TrackerDomainErrorCodes.AiProviderUnavailable);
        unavailable.Data["kind"].ShouldBe("rate-limited");
    }

    [Fact]
    public async Task An_Archived_Or_Unknown_Project_Should_Be_Refused_Before_Anything_Is_Sent()
    {
        await ReadyAsync();
        await WithUnitOfWorkAsync(async () =>
        {
            var projects = GetRequiredService<IRepository<Project, Guid>>();
            var website = await projects.GetAsync(WebsiteId);
            website.Archive(Day);
            await projects.UpdateAsync(website, autoSave: true);
        });

        (await Should.ThrowAsync<BusinessException>(() => Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal })))
            .Code.ShouldBe(TrackerDomainErrorCodes.ProjectArchived);
        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = Guid.NewGuid(), Goal = Goal }));
        Model.Calls.ShouldBeEmpty();
    }

    [Fact]
    public void The_Goal_Should_Be_Kept_Out_Of_The_Audit_Log()
    {
        typeof(AiTaskDraftsInput).GetProperty(nameof(AiTaskDraftsInput.Goal))!.GetCustomAttribute<DisableAuditingAttribute>().ShouldNotBeNull();

        var serialized = GetRequiredService<IAuditSerializer>().Serialize(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = Goal });

        serialized.ShouldNotContain(Goal);
        serialized.ShouldContain(WebsiteId.ToString());
    }
}

/// <summary>A worker of the Website who is a client of the Client portal (no Projects.Edit).</summary>
public abstract class TaskDraftsClientTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override string CallerPortalRole => TrackerRoles.ClientProjectRole;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(
            TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage,
            TrackerPermissions.Projects.Edit);
    }

    [Fact]
    public async Task A_Client_Of_The_Project_Should_Be_Refused_And_A_Worker_Of_Another_Served()
    {
        await ReadyAsync();

        (await Should.ThrowAsync<BusinessException>(() =>
                Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = PortalId, Goal = "Plan the portal launch" })))
            .Code.ShouldBe(TrackerDomainErrorCodes.RoleNotAllowed);
        Model.Calls.ShouldBeEmpty();

        Model.EnqueueJson(Json(new { drafts = new[] { new { title = "Plan the launch", description = "", rationale = "It starts the work.", duplicateOf = "" } } }));
        var result = await Ai.CreateTaskDraftsAsync(new AiTaskDraftsInput { ProjectId = WebsiteId, Goal = "Plan the launch" });

        result.Drafts.ShouldHaveSingleItem().Title.ShouldBe("Plan the launch");
    }
}
