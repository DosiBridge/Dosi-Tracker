using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json.Nodes;
using Dosi.Tracker.AiAssist.Output;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>
/// Builds an <see cref="AiContextBundle"/>. Ids are given in the order things are added, so callers add them in a
/// deterministic order (by minutes, then id) and the same data always gives the same bundle and hash.
/// <para>
/// Facts get <c>F1</c>…; their labels are server text without digits (a threshold is a fact of its own). Members get
/// handles from the <see cref="AiPseudonymizer"/> and keep their names only in the bundle's resolution. Projects,
/// apps, tasks, window titles and notes get <c>P</c>, <c>A</c>, <c>T</c>, <c>W</c> and <c>N</c> ids for their text,
/// sanitized by <see cref="UntrustedText"/>; the same project, task or app (or the same title or note text) keeps one id.
/// </para>
/// </summary>
public sealed class AiBundleWriter
{
    public const string FactPrefix = "F";
    public const string ProjectPrefix = "P";
    public const string AppPrefix = "A";
    public const string TaskPrefix = "T";
    public const string TitlePrefix = "W";
    public const string NotePrefix = "N";

    /// <summary>The caveat added once when an instruction-shaped phrase was removed from quoted text.</summary>
    public const string OmittedTextCaveat = "Some titles were omitted.";

    /// <summary>The text a label gets when nothing is left of it after sanitizing.</summary>
    public const string UntitledLabel = "(untitled)";

    private readonly AiPseudonymizer _pseudonymizer;
    private readonly List<KeyValuePair<string, AiFact>> _facts = new();
    private readonly List<KeyValuePair<string, string>> _labels = new();
    private readonly Dictionary<string, string> _labelIdsByKey = new(StringComparer.Ordinal);
    private readonly Dictionary<string, int> _labelCounts = new(StringComparer.Ordinal);
    private readonly JsonObject _lists = new();
    private readonly List<string> _caveats = new();
    private readonly List<string> _routes = new();
    private readonly JsonArray _candidates = new();
    private readonly Dictionary<string, AiAlias> _resolution = new(StringComparer.Ordinal);

    /// <summary>A writer for the caller's bundle, numbering members in order of first appearance.</summary>
    public AiBundleWriter(Guid callerId)
        : this(new AiPseudonymizer(callerId))
    {
    }

    /// <summary>A writer using these handles (<see cref="AiPseudonymizer.ForCard"/> for cards).</summary>
    public AiBundleWriter(AiPseudonymizer pseudonymizer)
    {
        _pseudonymizer = pseudonymizer;
    }

    public AiPseudonymizer Pseudonymizer => _pseudonymizer;

    /// <summary>How many quoted texts had an instruction-shaped phrase removed (for the metrics).</summary>
    public int InjectionRemovals { get; private set; }

    /// <summary>A fact; returns its id (<c>F1</c>…).</summary>
    /// <exception cref="ArgumentException">The label is blank or holds a digit (values belong in the display).</exception>
    public string AddFact(string label, string display)
    {
        if (string.IsNullOrWhiteSpace(label) || label.Any(char.IsDigit))
        {
            throw new ArgumentException("A fact label must be non-empty and hold no digits; put values in the display.", nameof(label));
        }

        var id = FactPrefix + (_facts.Count + 1).ToString(CultureInfo.InvariantCulture);
        _facts.Add(new KeyValuePair<string, AiFact>(id, new AiFact(label.Trim(), display ?? string.Empty)));
        return id;
    }

    /// <summary>The member's handle (<see cref="AiPseudonymizer.SelfHandle"/> for the caller); <paramref name="displayName"/>
    /// is kept only to render the answer.</summary>
    public string AddMember(Guid userId, string displayName)
    {
        var handle = _pseudonymizer.GetOrAdd(userId);
        _resolution[handle] = new AiAlias(userId, displayName ?? string.Empty);
        return handle;
    }

    /// <summary>The project's id (<c>P1</c>…) for its title.</summary>
    public string AddProject(Guid projectId, string? title) =>
        AddLabel(ProjectPrefix, projectId.ToString("N"), title, AiAssistConsts.MaxLabelLength);

    /// <summary>The app's id (<c>A1</c>…) for its canonical name (one id per name, whatever its case).</summary>
    public string AddApp(string? canonicalName) =>
        AddLabel(AppPrefix, (canonicalName ?? string.Empty).Trim().ToUpperInvariant(), canonicalName, AiAssistConsts.MaxLabelLength);

    /// <summary>The task's id (<c>T1</c>…) for its title.</summary>
    public string AddTask(Guid taskId, string? title) =>
        AddLabel(TaskPrefix, taskId.ToString("N"), title, AiAssistConsts.MaxLabelLength);

    /// <summary>A window title's id (<c>W1</c>…); the same sanitized text keeps one id.</summary>
    public string AddTitle(string? title) => AddLabel(TitlePrefix, null, title, AiAssistConsts.MaxTitleLength);

    /// <summary>A note's id (<c>N1</c>…); the same sanitized text keeps one id.</summary>
    public string AddNote(string? note) => AddLabel(NotePrefix, null, note, AiAssistConsts.MaxNoteLength);

    /// <summary>A named list (its values are ids); replaces a list of the same name. The node is copied.</summary>
    public AiBundleWriter SetList(string name, JsonNode? list)
    {
        if (string.IsNullOrWhiteSpace(name))
        {
            throw new ArgumentException("A list needs a name.", nameof(name));
        }

        _lists[name] = list?.DeepClone();
        return this;
    }

    /// <summary>A caveat the answer must respect (server text; added once).</summary>
    public AiBundleWriter AddCaveat(string caveat)
    {
        if (!string.IsNullOrWhiteSpace(caveat) && !_caveats.Contains(caveat))
        {
            _caveats.Add(caveat);
        }

        return this;
    }

    /// <summary>A route the answer may link to (added once).</summary>
    /// <exception cref="ArgumentException">The id is not in the <see cref="AiRouteIds"/> allowlist.</exception>
    public AiBundleWriter AddRoute(string routeId)
    {
        if (!AiRouteIds.IsAllowed(routeId))
        {
            throw new ArgumentException("Only allowlisted route ids may be offered.", nameof(routeId));
        }

        if (!_routes.Contains(routeId))
        {
            _routes.Add(routeId);
        }

        return this;
    }

    /// <summary>A rule-detected candidate (data-quality review). The node is copied.</summary>
    public AiBundleWriter AddCandidate(JsonObject candidate)
    {
        _candidates.Add(candidate.DeepClone());
        return this;
    }

    public AiContextBundle Build()
    {
        return new AiContextBundle(
            _pseudonymizer.CallerId,
            _facts.ToList(),
            (JsonObject)_lists.DeepClone(),
            _labels.ToList(),
            _caveats.ToList(),
            _routes.ToList(),
            (JsonArray)_candidates.DeepClone(),
            new Dictionary<string, AiAlias>(_resolution, StringComparer.Ordinal),
            InjectionRemovals);
    }

    private string AddLabel(string prefix, string? identity, string? raw, int max)
    {
        var text = UntrustedText.Sanitize(raw, max, out var injectionRemoved);
        if (injectionRemoved)
        {
            InjectionRemovals++;
            AddCaveat(OmittedTextCaveat);
        }

        if (text.Length == 0)
        {
            text = UntitledLabel;
        }

        // Things with an identity keep their id; free text (titles, notes) is one id per distinct text.
        var key = prefix + ":" + (identity ?? text);
        if (_labelIdsByKey.TryGetValue(key, out var existing))
        {
            return existing;
        }

        var number = _labelCounts.GetValueOrDefault(prefix) + 1;
        _labelCounts[prefix] = number;
        var id = prefix + number.ToString(CultureInfo.InvariantCulture);

        _labelIdsByKey[key] = id;
        _labels.Add(new KeyValuePair<string, string>(id, text));
        return id;
    }
}
