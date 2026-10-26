namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>data-quality-review@1 (spec 12.3): at most 5 items chosen from the server's rule candidates.</summary>
public static class DataQualityReviewPrompt
{
    public const int Version = 1;

    public const string Task = "Explain the most useful data-quality candidates in the data.";

    public const string Instructions =
        """
        Feature: data-quality review.
        - Use only data.candidates. Each candidate was found by a fixed rule; its kind says which, its member whom it is about, and its facts the figures behind it.
        - items: pick and order at most 5 candidates, the most useful first. For each:
          - candidateId: the candidate's id, exactly.
          - title: at most about 8 words.
          - explanation: one or two sentences on what the data shows.
          - possibleReasons: at most 3 short, benign reasons, such as leave, a device setting, remote desktop or accessibility tools.
          - actionId: one of that candidate's allowedActions, or "".
        - Data quality is about the tracker's data, never about people's conduct. Never write cheating, fraud, faking or slacking, and never imply intent.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "items":{"type":"array","items":{"type":"object","properties":{
            "candidateId":{"type":"string"},
            "title":{"type":"string"},
            "explanation":{"type":"string"},
            "possibleReasons":{"type":"array","items":{"type":"string"}},
            "actionId":{"type":"string"}},
            "required":["candidateId","title","explanation","possibleReasons","actionId"],"additionalProperties":false}}},
          "required":["items"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.DataQualityReview, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "data_quality_review");
}
