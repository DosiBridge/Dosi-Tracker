namespace Dosi.Tracker.AiAssist;

/// <summary>
/// Limits of AI assist: the ranges a feature may summarize, the size of what callers may type, how much of the tracker's
/// data one context bundle carries, and how long each part of a model's answer may be (longer output is cut, never
/// shown in full).
/// </summary>
public static class AiAssistConsts
{
    /// <summary>The version of the disclosure text this code ships. A workspace's consent records
    /// "{CodeDisclosureVersion}.{AI:Disclosure:Version}", so changing either asks owners to consent again.</summary>
    public const int CodeDisclosureVersion = 1;

    // Ranges, in days (inclusive).
    public const int MaxPeriodDays = 31;
    public const int MaxReportDays = 366;
    public const int MaxAssistantRangeDays = 92;
    public const int ComparePreviousMaxDays = 92;

    // What callers type.
    public const int MaxQuestionLength = 1000;
    public const int MaxGoalLength = 500;

    // How many items a feature returns.
    public const int MaxTaskDrafts = 8;
    public const int MaxNoteSuggestions = 3;
    public const int MaxNoteSuggestionLength = 200;

    // Text the tracker puts into a bundle (project, app and task names; notes; window titles).
    public const int MaxLabelLength = 120;
    public const int MaxNoteLength = 120;
    public const int MaxTitleLength = 120;

    // How much one bundle carries.
    public const int MaxBundleMembers = 12;
    public const int MaxBundleProjects = 10;
    public const int MaxBundleApps = 15;
    public const int MaxBundleDailyPoints = 31;

    // How long each part of an answer may be.
    public const int MaxHeadlineLength = 160;
    public const int MaxSummaryLength = 700;
    public const int MaxItemLength = 220;
    public const int MaxCaveatLength = 200;
    public const int MaxAnswerLength = 1200;
}
