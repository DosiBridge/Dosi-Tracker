using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Projects;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>One task draft as the model writes it.</summary>
public sealed record TaskDraftRaw(string Title, string Description, string Rationale, string DuplicateOf);

/// <summary>The task drafts as the model writes them.</summary>
public sealed record TaskDraftsRaw(IReadOnlyList<TaskDraftRaw?> Drafts);

/// <summary>
/// "Draft tasks with AI" (spec 11): drafts for one project, only for someone who could add its tasks
/// (<see cref="ProjectTaskRules.EnsureCanMutateProjectTasksAsync"/>: not archived, a project editor or a non-client
/// member). The bundle holds the project's title and description, its open (at most 50) and completed (at most 20) task
/// titles, and the caller's goal; no activity and no people. Creative output, clamped to the task limits; a draft may
/// flag an existing task it repeats. There is no deterministic answer: a failure is an error. Nothing is created: the
/// caller saves the drafts they keep. A new instance serves each request.
/// </summary>
public class TaskDraftsFeature : AiInsightFeatureBase<AiTaskDraftsInput, TaskDraftsRaw, AiTaskDraftsDto>, ITransientDependency
{
    public const int MaxOpenTasks = 50;
    public const int MaxCompletedTasks = 20;
    public const int MaxDescriptionLength = 600;
    public const int MaxRationaleLength = 160;

    public const string RequestList = "request";
    public const string ProjectList = "project";
    public const string GoalList = "goal";
    public const string ExistingTasksList = "existingTasks";

    private readonly IAiReadLayer _reads;
    private readonly ProjectTaskRules _taskRules;
    private readonly IReadOnlyRepository<Project, Guid> _projects;

    private AiProject? _project;
    private int _maxTasks = AiAssistConsts.MaxTaskDrafts;
    private readonly Dictionary<string, Guid> _taskIds = new(StringComparer.Ordinal);

    public TaskDraftsFeature(IAiReadLayer reads, ProjectTaskRules taskRules, IReadOnlyRepository<Project, Guid> projects)
    {
        _reads = reads;
        _taskRules = taskRules;
        _projects = projects;
    }

    public override AiPromptSpec Prompt => TaskDraftsPrompt.Spec;

    public override AiValidationProfile ValidationProfile => AiValidationProfile.Creative;

    public override bool HasFallback => false;

    public override string? TimeZoneOf(AiTaskDraftsInput input) => null;

    public override bool IsRegenerate(AiTaskDraftsInput input) => false;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiTaskDraftsInput input)
    {
        // A project the caller cannot see is a 404 first, so its state is never revealed; then the rules of adding a task.
        _project = await _reads.ProjectAsync(caller, input.ProjectId);
        await _taskRules.EnsureCanMutateProjectTasksAsync(await _projects.GetAsync(input.ProjectId));
        _maxTasks = Math.Clamp(input.MaxTasks, 1, AiAssistConsts.MaxTaskDrafts);
        return new AiAudienceScope(AiUsageRecord.ProjectAudience, ProjectId: _project.Id);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiTaskDraftsInput input, AiAudienceScope scope)
    {
        var project = _project!;
        var writer = new AiBundleWriter(caller.UserId);

        var details = new JsonObject { ["project"] = writer.AddProject(project.Id, project.Title) };
        if (!string.IsNullOrWhiteSpace(project.Description))
        {
            details["description"] = writer.AddDescription(project.Description);
        }

        writer.SetList(ProjectList, details);
        writer.SetList(GoalList, writer.AddGoal(input.Goal));
        writer.SetList(RequestList, new JsonObject { ["drafts"] = writer.AddFact("drafts requested", AiPeriodFacts.Count(_maxTasks)) });

        var open = await _reads.TasksAsync(caller, project.Id, isCompleted: false, MaxOpenTasks);
        var completed = await _reads.TasksAsync(caller, project.Id, isCompleted: true, MaxCompletedTasks);
        writer.SetList(ExistingTasksList, new JsonObject
        {
            ["open"] = TaskIds(writer, open.Items.Take(MaxOpenTasks)),
            ["completed"] = TaskIds(writer, completed.Items.Take(MaxCompletedTasks))
        });

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        AiFeatureOutput.Categories(bundle).Where(c => c != AiDataCategories.Durations).ToList();

    public override IReadOnlyCollection<string> QuotedInputs(AiContextBundle bundle) => bundle.Labels.Values.ToList();

    public override IReadOnlyList<AiViolation> Validate(TaskDraftsRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        var quoted = QuotedInputs(bundle);
        var tasks = bundle.Labels.Keys.Where(k => k.StartsWith(AiBundleWriter.TaskPrefix, StringComparison.Ordinal)).ToList();
        var index = 0;
        foreach (var draft in AiGroundingValidator.CapList(raw.Drafts, _maxTasks))
        {
            var at = $"drafts[{index++}]";
            if (draft == null)
            {
                violations.Add(new AiViolation(at, "is required"));
                continue;
            }

            AiFeatureOutput.ValidateText(violations, at + ".title", draft.Title, bundle, required: true, ValidationProfile, quoted);
            AiFeatureOutput.ValidateText(violations, at + ".description", draft.Description, bundle, required: false, ValidationProfile, quoted);
            AiFeatureOutput.ValidateText(violations, at + ".rationale", draft.Rationale, bundle, required: true, ValidationProfile, quoted);
            if (AiGroundingValidator.ValidateChoice(at + ".duplicateOf", draft.DuplicateOf, tasks, allowEmpty: true) is { } duplicate)
            {
                violations.Add(duplicate);
            }
        }

        return violations;
    }

    public override AiTaskDraftsDto Render(TaskDraftsRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Drafts = AiGroundingValidator.CapList(raw.Drafts?.Where(d => d != null && !string.IsNullOrWhiteSpace(d.Title)), _maxTasks)
            .Select(d =>
            {
                var description = AiFeatureOutput.RenderText(d!.Description, bundle,
                    Math.Min(MaxDescriptionLength, ProjectTaskConsts.MaxDescriptionLength));
                return new AiTaskDraftDto
                {
                    Title = AiFeatureOutput.RenderText(d.Title, bundle, ProjectTaskConsts.MaxTitleLength),
                    Description = description.Length == 0 ? null : description,
                    Rationale = AiFeatureOutput.RenderText(d.Rationale, bundle, MaxRationaleLength),
                    PossibleDuplicateOfTaskId = !string.IsNullOrEmpty(d.DuplicateOf) && _taskIds.TryGetValue(d.DuplicateOf, out var taskId)
                        ? taskId
                        : null
                };
            })
            .ToList()
    };

    public override AiTaskDraftsDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new() { Meta = meta };

    public override TimeSpan CacheTtl(AiTaskDraftsInput input, AiContextBundle bundle) => TimeSpan.FromHours(1);

    private JsonArray TaskIds(AiBundleWriter writer, IEnumerable<AiTask> tasks)
    {
        var ids = new JsonArray();
        foreach (var task in tasks)
        {
            var id = writer.AddTask(task.Id, task.Title);
            _taskIds[id] = task.Id;
            ids.Add(id);
        }

        return ids;
    }
}
