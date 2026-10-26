using System;
using System.Collections.Generic;
using Volo.Abp.Caching;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>One stored turn of a conversation: the pseudonymized question (recognized names already replaced by their
/// handles) and the rendered answer (its facts substituted with their values; member and project handles left as
/// placeholders, since they still resolve in a later turn of the same conversation). Never a real name.</summary>
public sealed class AiConversationTurn
{
    public string QuestionText { get; set; } = string.Empty;

    public string AnswerText { get; set; } = string.Empty;
}

/// <summary>
/// Ask AI's conversation memory (spec 15): the consent stamp it was started under, the handles given to members and
/// projects so far, and its last turns. <see cref="AiConversationStore"/> keeps it 60 minutes after each turn, capped
/// at 4 hours from <see cref="CreatedAtUtc"/> however busy the conversation stays. No real name, email or id of
/// anything but a project ever appears here: members are keyed by their handle only.
/// </summary>
[Serializable]
[CacheName("Tracker.AiConversation")]
public class AiConversationCacheItem
{
    public DateTime CreatedAtUtc { get; set; }

    /// <summary>EnabledAt ticks, consent version and the window-title/notes flags, combined: a change here (the owner
    /// turned AI off and on again, re-consented, or changed what is shared) ends the conversation.</summary>
    public string ConsentStamp { get; set; } = string.Empty;

    /// <summary>Member handles given so far ("M1", "M2" …), by user id. The caller is never in this map (they are
    /// always {ME}).</summary>
    public Dictionary<Guid, string> MemberHandles { get; set; } = new();

    /// <summary>Project handles given so far ("P1", "P2" …), by project id.</summary>
    public Dictionary<Guid, string> ProjectRefs { get; set; } = new();

    /// <summary>The last turns, oldest first, at most <c>AI:Assistant:MaxHistoryTurns</c>.</summary>
    public List<AiConversationTurn> Turns { get; set; } = new();
}
