namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>note-suggestions@1 (spec 12.3, creative profile): at most 3 short notes for one block.</summary>
public static class NoteSuggestionsPrompt
{
    public const int Version = 1;

    public const string Task = "Suggest short notes for the tracked block in the data.";

    public const string Instructions =
        """
        Feature: note suggestions for one block of tracked time.
        - suggestions: at most 3 items, each a different note of at most about 12 words, in the past tense or as an imperative ("Reviewed ...", "Fix ...").
        - basis says what a suggestion is based on: apps, project, window-titles (only when data.lists.windowTitles exists) or previous-note (only when data.lists.notes.previous exists).
        - Use only what the data says about this block: its project, its apps, and its window titles and notes when present. You may write the quoted names through their placeholders ({P1}, {A1}, {W1}) or in your own words.
        - Never name or refer to a person, and never use {ME} or a member placeholder.
        - This rule replaces rule 2 of the policy for this feature: you may repeat a number only when it appears in the quoted names.
        """;

    public const string Schema =
        """
        {"type":"object","properties":{
          "suggestions":{"type":"array","items":{"type":"object","properties":{
            "text":{"type":"string"},
            "basis":{"type":"string","enum":["apps","project","window-titles","previous-note"]}},
            "required":["text","basis"],"additionalProperties":false}}},
          "required":["suggestions"],"additionalProperties":false}
        """;

    public static readonly AiPromptSpec Spec = new(
        AiFeatureIds.NoteSuggestions, Version, Task, Instructions, AiPromptSpec.ParseSchema(Schema), "note_suggestions");
}
