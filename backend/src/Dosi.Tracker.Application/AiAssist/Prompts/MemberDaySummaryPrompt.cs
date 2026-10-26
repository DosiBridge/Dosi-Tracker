namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>member-day-summary@1 (spec 12.3): a headline, a short summary, at most 5 highlights and 3 caveats.</summary>
public static class MemberDaySummaryPrompt
{
    public const int Version = 1;

    public const string Task = "Describe the tracked work of the day in the data.";

    public const string Instructions =
        """
        Feature: day summary.
        - headline: one sentence of at most about 15 words.
        - summary: two to four sentences covering when the day started and ended, the main projects and apps, the focus stretches, the breaks, and the change against the average of the previous week when the data states it.
        - highlights: at most 5 items. kind is one of focus, apps, projects, breaks, rhythm, data-quality. text is one sentence. factRefs lists the ids of the facts the text uses.
        - caveats: at most 3 short sentences, taken from data.caveats when they affect what you wrote.
        - data.lists.totals names the facts of the day (tracked, idle, activity, firstStart, lastEnd, longestSpan, breaks ...). data.lists.projects and data.lists.apps hold the time per project and app, data.lists.dayParts the shares of the morning, afternoon and evening, data.lists.endReasons how blocks ended. Window titles and notes appear only when the workspace allows them.
        - Write about the member's day only; do not guess what the work was beyond the quoted names.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "headline":{"type":"string"},
          "summary":{"type":"string"},
          "highlights":{"type":"array","items":{"type":"object","properties":{
            "kind":{"type":"string","enum":["focus","apps","projects","breaks","rhythm","data-quality"]},
            "text":{"type":"string"},
            "factRefs":{"type":"array","items":{"type":"string"}}},
            "required":["kind","text","factRefs"],"additionalProperties":false}},
          "caveats":{"type":"array","items":{"type":"string"}}},
          "required":["headline","summary","highlights","caveats"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.MemberDaySummary, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "member_day_summary");
}
