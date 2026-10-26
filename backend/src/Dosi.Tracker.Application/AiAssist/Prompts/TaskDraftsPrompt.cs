namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>task-drafts@1 (spec 12.3, creative profile): task drafts for one project and goal.</summary>
public static class TaskDraftsPrompt
{
    public const int Version = 1;

    public const string Task = "Draft tasks for the project and goal in the data.";

    public const string Instructions =
        """
        Feature: task drafts for one project.
        - The goal is the quoted G1 text; the project is P1, with its description in D1 when present. data.lists.existingTasks lists the project's open and completed tasks.
        - drafts: at most as many items as data.lists.request.drafts says. For each:
          - title: starts with a verb, at most about 10 words.
          - description: one to three sentences, or "".
          - rationale: one sentence on how the task serves the goal.
          - duplicateOf: the id of an existing task (for example "T3") this draft may repeat, or "".
        - Do not repeat existing tasks unless you flag them with duplicateOf.
        - No estimates, dates, assignees or people. Never use {ME} or a member placeholder.
        - This rule replaces rule 2 of the policy for this feature: write plain words, and repeat a number only when it appears in the quoted texts.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "drafts":{"type":"array","items":{"type":"object","properties":{
            "title":{"type":"string"},
            "description":{"type":"string"},
            "rationale":{"type":"string"},
            "duplicateOf":{"type":"string"}},
            "required":["title","description","rationale","duplicateOf"],"additionalProperties":false}}},
          "required":["drafts"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.TaskDrafts, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "task_drafts");
}
