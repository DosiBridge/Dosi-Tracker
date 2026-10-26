namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>report-explanation@1 (spec 12.3): a summary, at most 6 key findings, 4 caveats and 3 questions.</summary>
public static class ReportExplanationPrompt
{
    public const int Version = 1;

    public const string Task = "Explain what the report in the data shows.";

    public const string Instructions =
        """
        Feature: report explanation.
        - data.lists.report names the report. summary: two to four sentences on what the report shows for its range.
        - keyFindings: at most 6 items. kind is one of trend, distribution, coverage, rhythm, attendance, apps, devices, data-quality. text is one or two sentences. factRefs lists the ids of the facts the text uses. routeId is one of data.routes, or "" for none.
        - caveats: at most 4 short sentences, taken from data.caveats when they affect what you wrote. In attendance, absent means no tracked activity on a working day, nothing more.
        - questionsToExplore: at most 3 short, neutral questions the reader could look into in the product. Never an accusation and never about a person's character.
        - data.lists.totals names the facts of the whole range. Rows of the report are in data.lists (members, projects, apps, days, weeks or months, hours, attendance, devices); members beyond the listed ones are folded into data.lists.otherMembers.
        - Describe distributions and changes; do not rank people.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "summary":{"type":"string"},
          "keyFindings":{"type":"array","items":{"type":"object","properties":{
            "kind":{"type":"string","enum":["trend","distribution","coverage","rhythm","attendance","apps","devices","data-quality"]},
            "text":{"type":"string"},
            "factRefs":{"type":"array","items":{"type":"string"}},
            "routeId":{"type":"string"}},
            "required":["kind","text","factRefs","routeId"],"additionalProperties":false}},
          "caveats":{"type":"array","items":{"type":"string"}},
          "questionsToExplore":{"type":"array","items":{"type":"string"}}},
          "required":["summary","keyFindings","caveats","questionsToExplore"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.ReportExplanation, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "report_explanation");
}
