using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.AiAssist;

/// <summary>The page the member asked from, so the answer can be grounded in what they are looking at.</summary>
public class AiPageContextDto
{
    /// <summary>"dashboard", "monitor", "timesheet", "reports", "projects", "activities", "team", "settings",
    /// "download" or "other".</summary>
    public string? Page { get; set; }

    /// <summary>One of <see cref="Read.AiScopeGuard.ReportSlugs"/>, when <see cref="Page"/> is "reports".</summary>
    public string? ReportSlug { get; set; }

    public DateTime? From { get; set; }

    public DateTime? To { get; set; }

    public Guid? ProjectId { get; set; }

    public Guid? MemberId { get; set; }

    /// <summary>yyyy-MM-dd.</summary>
    public string? Day { get; set; }

    public string? TimeZone { get; set; }
}

/// <summary>One turn of Ask AI (POST api/app/ai-assist/ask).</summary>
public class AiAskInput
{
    /// <summary>Null starts a new conversation.</summary>
    public Guid? ConversationId { get; set; }

    [Required]
    [StringLength(AiAssistConsts.MaxQuestionLength, MinimumLength = 1)]
    [DisableAuditing]
    public string Message { get; set; } = string.Empty;

    public AiPageContextDto? Context { get; set; }
}

/// <summary>Where one part of the answer came from, for "show the figures used".</summary>
public class AiSourceDto
{
    /// <summary>What the tool call found ("your tracked time", "the team's app usage" ...).</summary>
    public string Label { get; set; } = string.Empty;

    /// <summary>An allowlisted route id the web app may link to, or null.</summary>
    public string? RouteId { get; set; }

    /// <summary>The route's argument (a project id), when it needs one.</summary>
    public string? RouteArg { get; set; }
}

/// <summary>One turn's answer.</summary>
public class AiAnswerDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    /// <summary>Pass this back as the next turn's <see cref="AiAskInput.ConversationId"/>.</summary>
    public Guid ConversationId { get; set; }

    /// <summary>Rendered plain text; lines starting "- " are list items.</summary>
    public string Answer { get; set; } = string.Empty;

    /// <summary>At most 6.</summary>
    public List<AiSourceDto> Sources { get; set; } = new();

    /// <summary>At most 20.</summary>
    public List<AiFactRefDto> Facts { get; set; } = new();

    /// <summary>True when the answer only asks the member to say which person or project they meant (no model call,
    /// no tokens, no tool calls).</summary>
    public bool IsClarification { get; set; }
}
