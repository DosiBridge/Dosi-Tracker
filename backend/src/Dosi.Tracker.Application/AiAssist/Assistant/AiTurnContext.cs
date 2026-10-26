using System;
using System.Collections.Generic;
using System.Globalization;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Pipeline;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>Accumulates the token usage a turn's provider calls reported, across every round
/// (<see cref="UsageMeteringChatClient"/>); thread-safe because <c>AllowConcurrentInvocation</c> is off but rounds
/// still run one after another on the same instance.</summary>
public sealed class AiTurnUsage
{
    private readonly object _lock = new();
    private AiTokenUsage _usage = AiTokenUsage.None;

    public AiTokenUsage Snapshot
    {
        get
        {
            lock (_lock)
            {
                return _usage;
            }
        }
    }

    public void Add(UsageDetails? details)
    {
        lock (_lock)
        {
            _usage = _usage.Add(details);
        }
    }
}

/// <summary>One tool result, kept to build the answer's sources: its label, the route it may offer, and the facts
/// (and other ids) it put in play.</summary>
public sealed record AiCitation(string ResultId, string Tool, string Label, string? RouteId, string? RouteArg);

/// <summary>
/// Everything one Ask AI turn carries while its tools run (spec 9.2, 10.2): who is asking, the conversation's handles
/// (members and projects keep theirs across turns), this turn's facts, its call and character budgets, the citations
/// its tool results earned, the token usage its provider calls add up to, and (render-only, never sent to the model)
/// the real names and quoted text behind every handle, so the final answer can be rendered for the reader. Built
/// fresh for every <c>AskAsync</c> call; never reused between requests.
/// </summary>
public sealed class AiTurnContext
{
    private readonly Dictionary<string, Guid> _projectsByHandle = new(StringComparer.Ordinal);
    private readonly Dictionary<Guid, string> _projectHandles = new();
    private readonly Dictionary<string, string> _appHandles = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<Guid, string> _taskHandles = new();
    private readonly List<AiCitation> _citations = new();
    private readonly Dictionary<Guid, string> _memberNames = new();
    private readonly Dictionary<string, string> _labels = new(StringComparer.Ordinal);

    private int _projectSequence;
    private int _appSequence;
    private int _taskSequence;
    private int _resultSequence;
    private int _calls;
    private int _turnChars;

    public AiTurnContext(
        AiCallerContext caller, IReadOnlyDictionary<Guid, string>? memberHandles, IReadOnlyDictionary<Guid, string>? projectRefs = null)
    {
        Caller = caller;
        Members = AiPseudonymizer.ForConversation(caller.UserId, memberHandles);

        _projectSequence = 0;
        foreach (var (projectId, handle) in projectRefs ?? new Dictionary<Guid, string>())
        {
            if (!TryParseNumber("P", handle, out var number) || _projectsByHandle.ContainsKey(handle))
            {
                continue;
            }

            _projectHandles[projectId] = handle;
            _projectsByHandle[handle] = projectId;
            _projectSequence = Math.Max(_projectSequence, number);
            Facts.Register(handle);
        }

        foreach (var handle in Members.Handles.Values)
        {
            Facts.Register(handle);
        }
    }

    public AiCallerContext Caller { get; }

    /// <summary>Member handles: the conversation's, kept; newcomers numbered in order of first appearance.</summary>
    public AiPseudonymizer Members { get; }

    public AiFactRegistry Facts { get; } = new();

    public AiTurnUsage Usage { get; } = new();

    /// <summary>Tool results that may become sources, in the order their calls returned.</summary>
    public IReadOnlyList<AiCitation> Citations => _citations;

    /// <summary>Every project handle known this turn (the conversation's plus newly minted ones), for persisting.</summary>
    public IReadOnlyDictionary<Guid, string> ProjectRefs => _projectHandles;

    /// <summary>Real display names behind the member handles used this turn (render-only; never sent anywhere).</summary>
    public IReadOnlyDictionary<Guid, string> MemberNames => _memberNames;

    /// <summary>Sanitized quoted text behind the project, app and task handles used this turn (render-only; never
    /// sent as such — the model already saw it inline in a tool result's own <c>_text</c> field).</summary>
    public IReadOnlyDictionary<string, string> Labels => _labels;

    public int CallsMade => _calls;

    public int TurnCharsUsed => _turnChars;

    /// <summary>A fresh result id ("c1", "c2" …).</summary>
    public string NextResultId() => "c" + (++_resultSequence).ToString(CultureInfo.InvariantCulture);

    /// <summary>One more tool call is about to run; false (no handler runs) once the turn's call budget is spent.</summary>
    public bool TryReserveCall(int maxCalls) => ++_calls <= Math.Max(0, maxCalls);

    /// <summary>Reserves <paramref name="chars"/> more of the turn's character budget; false (the result is dropped)
    /// once it would be spent.</summary>
    public bool TryReserveChars(int chars, int maxTurnChars)
    {
        if (_turnChars + chars > Math.Max(0, maxTurnChars))
        {
            return false;
        }

        _turnChars += chars;
        return true;
    }

    /// <summary>The project's handle ("P1" …), minted on first appearance (this turn or an earlier one in the same
    /// conversation). <paramref name="title"/>, if given, is kept (render-only) the first time it is seen.</summary>
    public string AddProjectRef(Guid projectId, string? title = null)
    {
        if (!_projectHandles.TryGetValue(projectId, out var handle))
        {
            handle = "P" + (++_projectSequence).ToString(CultureInfo.InvariantCulture);
            _projectHandles[projectId] = handle;
            _projectsByHandle[handle] = projectId;
            Facts.Register(handle);
        }

        SetLabel(handle, title);
        return handle;
    }

    /// <summary>The project a handle ("P2") names this turn, or null.</summary>
    public Guid? ResolveProjectRef(string? handle) =>
        !string.IsNullOrEmpty(handle) && _projectsByHandle.TryGetValue(handle, out var id) ? id : null;

    /// <summary>An app's handle ("A1" …) for this turn, one per canonical name.</summary>
    public string AddAppRef(string canonicalName)
    {
        var key = canonicalName.Trim();
        if (!_appHandles.TryGetValue(key, out var handle))
        {
            handle = "A" + (++_appSequence).ToString(CultureInfo.InvariantCulture);
            _appHandles[key] = handle;
            Facts.Register(handle);
        }

        SetLabel(handle, key);
        return handle;
    }

    /// <summary>A task's handle ("T1" …) for this turn.</summary>
    public string AddTaskRef(Guid taskId, string? title = null)
    {
        if (!_taskHandles.TryGetValue(taskId, out var handle))
        {
            handle = "T" + (++_taskSequence).ToString(CultureInfo.InvariantCulture);
            _taskHandles[taskId] = handle;
            Facts.Register(handle);
        }

        SetLabel(handle, title);
        return handle;
    }

    /// <summary>The member's handle ("ME" for the caller, "M1" … for anyone else), minted on first appearance (this
    /// turn or an earlier one) and registered as a known placeholder id. <paramref name="displayName"/>, if given, is
    /// kept (render-only) the first time it is seen.</summary>
    public string AddMemberRef(Guid userId, string? displayName = null)
    {
        var handle = userId == Caller.UserId ? AiPseudonymizer.SelfHandle : Members.GetOrAdd(userId);
        Facts.Register(handle);
        if (!_memberNames.ContainsKey(userId))
        {
            var sanitized = UntrustedText.Sanitize(displayName, AiAssistConsts.MaxLabelLength);
            if (sanitized.Length > 0)
            {
                _memberNames[userId] = sanitized;
            }
        }

        return handle;
    }

    /// <summary>The member a handle ("M3" or "ME") names, or null for an unknown one.</summary>
    public Guid? ResolveMemberRef(string? handle)
    {
        if (string.IsNullOrEmpty(handle))
        {
            return null;
        }

        if (handle == AiPseudonymizer.SelfHandle)
        {
            return Caller.UserId;
        }

        return Members.TryResolve(handle, out var userId) ? userId : null;
    }

    public void AddCitation(AiCitation citation) => _citations.Add(citation);

    /// <summary>Keeps the first text given for a handle, sanitized: the final answer renders this, so whatever a
    /// caller passed in (even if it skipped sanitizing before quoting it to the model) can never carry raw quoted
    /// text, a forged placeholder or an instruction-shaped phrase into what the reader sees.</summary>
    private void SetLabel(string handle, string? text)
    {
        if (_labels.ContainsKey(handle))
        {
            return;
        }

        var sanitized = UntrustedText.Sanitize(text, AiAssistConsts.MaxLabelLength);
        if (sanitized.Length > 0)
        {
            _labels[handle] = sanitized;
        }
    }

    private static bool TryParseNumber(string prefix, string? handle, out int number)
    {
        number = 0;
        return handle is { Length: > 1 } &&
               handle.StartsWith(prefix, StringComparison.Ordinal) &&
               int.TryParse(handle.AsSpan(prefix.Length), NumberStyles.None, CultureInfo.InvariantCulture, out number) &&
               number > 0;
    }
}
