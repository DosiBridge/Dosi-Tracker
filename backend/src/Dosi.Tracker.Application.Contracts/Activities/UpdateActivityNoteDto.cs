using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Activities;

/// <summary>The task note ("What are you working on?") of a tracked block.</summary>
public class UpdateActivityNoteDto
{
    public const int MaxDescriptionLength = 2048;

    /// <summary>Stored trimmed, without control characters and with runs of whitespace collapsed to one space;
    /// null or blank clears the note.</summary>
    [MaxLength(MaxDescriptionLength)]
    public string? Description { get; set; }
}
