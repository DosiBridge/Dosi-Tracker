using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.AiAssist;

/// <summary>Note suggestions for one block (<c>POST api/app/ai-assist/note-suggestions</c>). Nothing is saved: the
/// member picks or edits a suggestion and saves it with <c>PUT api/app/activity/{id}/note</c>.</summary>
public class AiNoteSuggestionsInput
{
    [Required]
    public Guid ActivityId { get; set; }
}

public class AiNoteSuggestionsDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    /// <summary>At most 3.</summary>
    public List<AiNoteSuggestionDto> Suggestions { get; set; } = new();
}

public class AiNoteSuggestionDto
{
    /// <summary>At most <see cref="AiAssistConsts.MaxNoteSuggestionLength"/> characters.</summary>
    public string Text { get; set; } = string.Empty;

    /// <summary>What it is based on: one of <see cref="AiNoteSuggestionBases.All"/>.</summary>
    public string Basis { get; set; } = string.Empty;
}

public static class AiNoteSuggestionBases
{
    public const string Apps = "apps";
    public const string Project = "project";
    public const string WindowTitles = "window-titles";
    public const string PreviousNote = "previous-note";

    public static readonly IReadOnlyList<string> All = new[] { Apps, Project, WindowTitles, PreviousNote };
}
