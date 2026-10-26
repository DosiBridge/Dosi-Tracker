namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>period-summary@1 (spec 12.3): a headline, at most 5 insights, 3 next steps and 3 caveats.</summary>
public static class PeriodSummaryPrompt
{
    public const int Version = 1;

    public const string Task = "Summarize the tracked work of the period in the data for its reader.";

    public const string Instructions =
        """
        Feature: period summary.
        - headline: one sentence of at most about 15 words.
        - insights: at most 5 items. kind is one of trend, distribution, coverage, rhythm, data-quality. text is one or two sentences. factRefs lists the ids of the facts the text uses, for example "F3". routeId is one of data.routes, or "" for none.
        - nextSteps: at most 3 items, each a short label and a routeId from data.routes.
        - caveats: at most 3 short sentences, taken from data.caveats when they affect what you wrote.
        - data.lists.totals names the facts of the whole period (tracked, idle, activity, previousTracked, change ...). data.lists.days holds the tracked time per day, data.lists.projects and data.lists.apps the largest projects and apps, data.lists.members (when present) the tracked time per member, with everyone else folded into data.lists.otherMembers.
        - Mention members only in statements about how tracked time is distributed. Never single out a member negatively.
        - When the data holds only totals, write the headline and leave insights empty.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "headline":{"type":"string"},
          "insights":{"type":"array","items":{"type":"object","properties":{
            "kind":{"type":"string","enum":["trend","distribution","coverage","rhythm","data-quality"]},
            "text":{"type":"string"},
            "factRefs":{"type":"array","items":{"type":"string"}},
            "routeId":{"type":"string"}},
            "required":["kind","text","factRefs","routeId"],"additionalProperties":false}},
          "nextSteps":{"type":"array","items":{"type":"object","properties":{
            "label":{"type":"string"},
            "routeId":{"type":"string"}},
            "required":["label","routeId"],"additionalProperties":false}},
          "caveats":{"type":"array","items":{"type":"string"}}},
          "required":["headline","insights","nextSteps","caveats"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.PeriodSummary, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "period_summary");
}
