using System;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.AiAssist;

/// <summary>Helpful / not helpful on one AI result (<c>POST api/app/ai-assist/feedback</c>), by the member who received
/// it.</summary>
public class AiFeedbackInput
{
    /// <summary>The result's <see cref="AiResultMetaDto.ResultId"/>.</summary>
    [Required]
    public Guid ResultId { get; set; }

    /// <summary>"up" or "down".</summary>
    [Required]
    [RegularExpression("^(up|down)$")]
    public string Rating { get; set; } = string.Empty;

    /// <summary>"inaccurate", "not-useful", "tone", "other", or null.</summary>
    [RegularExpression("^(inaccurate|not-useful|tone|other)$")]
    public string? Reason { get; set; }
}
