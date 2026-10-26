namespace Dosi.Tracker.AiAssist.Output;

/// <summary>Which output rules apply (<see cref="AiGroundingValidator"/>).</summary>
public enum AiValidationProfile
{
    /// <summary>Cards that describe data: every number, date, weekday or month only as a fact placeholder.</summary>
    Grounded,

    /// <summary>Note suggestions and task drafts: free wording; digits only when quoted from the inputs, and no
    /// people (no member or reader placeholders).</summary>
    Creative,

    /// <summary>Ask AI answers: the grounded rules on plain text, at most <see cref="AiAssistConsts.MaxAnswerLength"/>
    /// characters, no headings.</summary>
    Chat
}

/// <summary>One broken output rule: where (a JSON path such as <c>insights[1].text</c>) and what, in words that never
/// quote the output itself.</summary>
public sealed record AiViolation(string Path, string Message);
