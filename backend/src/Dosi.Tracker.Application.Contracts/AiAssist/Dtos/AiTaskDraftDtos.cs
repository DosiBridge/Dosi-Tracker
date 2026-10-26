using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.AiAssist;

/// <summary>Task drafts for a project (<c>POST api/app/ai-assist/task-drafts</c>). Nothing is created: the caller
/// reviews the drafts and saves the ones they keep with <c>POST api/app/project-task</c>.</summary>
public class AiTaskDraftsInput
{
    [Required]
    public Guid ProjectId { get; set; }

    /// <summary>What the tasks should achieve. Free text, so it is kept out of the audit log.</summary>
    [Required]
    [StringLength(AiAssistConsts.MaxGoalLength, MinimumLength = 3)]
    [DisableAuditing]
    public string Goal { get; set; } = string.Empty;

    [Range(1, AiAssistConsts.MaxTaskDrafts)]
    public int MaxTasks { get; set; } = 5;
}

public class AiTaskDraftsDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    /// <summary>At most <see cref="AiTaskDraftsInput.MaxTasks"/>.</summary>
    public List<AiTaskDraftDto> Drafts { get; set; } = new();
}

public class AiTaskDraftDto
{
    /// <summary>At most 256 characters.</summary>
    public string Title { get; set; } = string.Empty;

    /// <summary>At most 600 characters, or null.</summary>
    public string? Description { get; set; }

    /// <summary>Why the task helps (at most 160 characters).</summary>
    public string Rationale { get; set; } = string.Empty;

    /// <summary>An existing task of the project this draft may repeat.</summary>
    public Guid? PossibleDuplicateOfTaskId { get; set; }
}
