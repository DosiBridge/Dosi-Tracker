using System;
using Volo.Abp.Caching;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// One cached AI answer: the validated raw output as the model wrote it, so still pseudonymized (placeholders and
/// handles, never a real name), plus what it cost. Rendering always uses the reader's current bundle.
/// </summary>
[Serializable]
[CacheName("Tracker.AiResult")]
public class AiCachedResultItem
{
    /// <summary>The raw answer as camelCase JSON.</summary>
    public string RawJson { get; set; } = string.Empty;

    public string? Model { get; set; }

    public string PromptVersion { get; set; } = string.Empty;

    public int? InputTokens { get; set; }

    public int? OutputTokens { get; set; }

    /// <summary>When the model wrote it (UTC).</summary>
    public DateTime GeneratedAt { get; set; }
}
