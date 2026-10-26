using System;
using System.Collections.Generic;
using System.Globalization;
using Dosi.Tracker.AiAssist.Context;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// The facts (<c>F1</c>, <c>F2</c> …) one Ask AI turn adds while its tools run, each tied to the tool result
/// (<c>c1</c>, <c>c2</c> …) it came from so the answer's sources can be built. Facts never persist across turns: the
/// validator and the assistant prompt both require every number to come from a tool called in this turn.
/// </summary>
public sealed class AiFactRegistry
{
    private readonly List<KeyValuePair<string, AiFact>> _facts = new();
    private readonly Dictionary<string, string> _factResults = new(StringComparer.Ordinal);
    private readonly HashSet<string> _knownIds = new(StringComparer.Ordinal) { AiPseudonymizer.SelfHandle };

    /// <summary>Every fact added this turn, by id.</summary>
    public IReadOnlyDictionary<string, AiFact> Facts => Facts2;

    private Dictionary<string, AiFact> Facts2
    {
        get
        {
            var map = new Dictionary<string, AiFact>(StringComparer.Ordinal);
            foreach (var pair in _facts)
            {
                map[pair.Key] = pair.Value;
            }

            return map;
        }
    }

    /// <summary>Every id a placeholder may name this turn (facts, plus ids registered through <see cref="Register"/>).</summary>
    public IReadOnlySet<string> KnownIds => _knownIds;

    /// <summary>A fact from one tool result; returns its id ("F1" …).</summary>
    /// <param name="label">What the value is (server text without digits).</param>
    /// <param name="display">The formatted value.</param>
    /// <param name="resultId">The tool result ("c1" …) this fact came from.</param>
    public string AddFact(string label, string display, string resultId)
    {
        if (string.IsNullOrWhiteSpace(label) || HasDigit(label))
        {
            throw new ArgumentException("A fact label must be non-empty and hold no digits.", nameof(label));
        }

        var id = "F" + (_facts.Count + 1).ToString(CultureInfo.InvariantCulture);
        _facts.Add(new KeyValuePair<string, AiFact>(id, new AiFact(label.Trim(), display ?? string.Empty)));
        _factResults[id] = resultId;
        _knownIds.Add(id);
        return id;
    }

    /// <summary>Registers an id (a member handle, a project, app, task, title or note id) as known this turn, so the
    /// validator accepts placeholders naming it.</summary>
    public void Register(string id)
    {
        if (!string.IsNullOrWhiteSpace(id))
        {
            _knownIds.Add(id);
        }
    }

    /// <summary>The tool result ("c1" …) a fact came from, or null.</summary>
    public string? ResultOf(string factId) => _factResults.TryGetValue(factId, out var resultId) ? resultId : null;

    private static bool HasDigit(string text)
    {
        foreach (var c in text)
        {
            if (char.IsDigit(c))
            {
                return true;
            }
        }

        return false;
    }
}
