using System;
using System.Collections.Generic;
using System.Linq;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The kinds of tracker data an AI request may send to the provider. Every result lists the categories it used, the
/// usage ledger records them, and the disclosure names them. Screenshots, webcam images, keystrokes and input counts
/// are no category: they are never sent.
/// </summary>
public static class AiDataCategories
{
    public const string Durations = "durations";
    public const string Times = "times";

    /// <summary>The share of minutes with keyboard or mouse input; never the counts themselves.</summary>
    public const string ActivityPercent = "activity-percent";

    public const string ProjectNames = "project-names";
    public const string TaskTitles = "task-titles";

    /// <summary>Notes people wrote on their time; sent only when the workspace allows notes.</summary>
    public const string TaskNotes = "task-notes";

    public const string AppNames = "app-names";

    /// <summary>Window titles; sent only when the workspace allows them.</summary>
    public const string WindowTitles = "window-titles";

    /// <summary>Agent version, platform and whether a session could measure input; never device names.</summary>
    public const string AgentInfo = "agent-info";

    /// <summary>The product's own help text (Ask AI).</summary>
    public const string HelpContent = "help-content";

    public static readonly IReadOnlyList<string> All = new[]
    {
        Durations, Times, ActivityPercent, ProjectNames, TaskTitles, TaskNotes, AppNames, WindowTitles, AgentInfo, HelpContent
    };

    public static bool IsKnown(string? category) => category != null && All.Contains(category, StringComparer.Ordinal);
}
