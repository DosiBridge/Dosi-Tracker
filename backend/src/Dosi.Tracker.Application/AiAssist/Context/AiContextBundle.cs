using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>A value the model may cite: what it is (a label without digits) and how it reads ("6 h 52 min").</summary>
public sealed record AiFact(string Label, string Display);

/// <summary>Who a member handle stands for. Held in memory only, to render answers; never sent or serialized.</summary>
public sealed record AiAlias(Guid UserId, string DisplayName);

/// <summary>
/// Everything one AI request may tell the model, already minimized and pseudonymized. Built by
/// <see cref="AiBundleWriter"/>; sent as two blocks by <see cref="AiCanonicalJson"/> (the data, and the quoted labels).
/// <para>
/// People appear only as handles (<c>M1</c>…, the reader as <c>ME</c>); projects, apps, tasks, window titles and notes
/// as ids (<c>P1</c>, <c>A1</c>, <c>T1</c>, <c>W1</c>, <c>N1</c>) whose sanitized text is in <see cref="Labels"/>; values as
/// facts (<c>F1</c>…). Real names exist only in <see cref="Resolution"/>.
/// </para>
/// </summary>
public sealed class AiContextBundle
{
    internal AiContextBundle(
        Guid callerId,
        IReadOnlyList<KeyValuePair<string, AiFact>> facts,
        JsonObject lists,
        IReadOnlyList<KeyValuePair<string, string>> labels,
        IReadOnlyList<string> caveats,
        IReadOnlyList<string> routes,
        JsonArray candidates,
        IReadOnlyDictionary<string, AiAlias> resolution)
    {
        CallerId = callerId;
        FactList = facts;
        Facts = facts.ToDictionary(f => f.Key, f => f.Value, StringComparer.Ordinal);
        Lists = lists;
        LabelList = labels;
        Labels = labels.ToDictionary(l => l.Key, l => l.Value, StringComparer.Ordinal);
        Caveats = caveats;
        Routes = routes;
        Candidates = candidates;
        Resolution = resolution;
        KnownIds = new HashSet<string>(
            Facts.Keys.Concat(Labels.Keys).Concat(Resolution.Keys).Append(AiPseudonymizer.SelfHandle),
            StringComparer.Ordinal);
    }

    /// <summary>The reader (the signed-in caller), whose handle is <see cref="AiPseudonymizer.SelfHandle"/>.</summary>
    public Guid CallerId { get; }

    /// <summary>Facts by id (<c>F1</c>…).</summary>
    public IReadOnlyDictionary<string, AiFact> Facts { get; }

    /// <summary>The facts in the order they were added.</summary>
    public IReadOnlyList<KeyValuePair<string, AiFact>> FactList { get; }

    /// <summary>Named lists (daily points, members, projects, apps …) whose values are ids, never raw text or numbers.
    /// Read-only by convention: change a bundle through its writer only.</summary>
    public JsonObject Lists { get; }

    /// <summary>Sanitized quoted text by id (<c>P1</c>, <c>A1</c>, <c>T1</c>, <c>W1</c>, <c>N1</c>).</summary>
    public IReadOnlyDictionary<string, string> Labels { get; }

    /// <summary>The labels in the order they were added.</summary>
    public IReadOnlyList<KeyValuePair<string, string>> LabelList { get; }

    /// <summary>Server-written notes the text must respect ("Activity % was not measured for part of the period").</summary>
    public IReadOnlyList<string> Caveats { get; }

    /// <summary>The route ids (<see cref="Output.AiRouteIds"/>) the answer may link to.</summary>
    public IReadOnlyList<string> Routes { get; }

    /// <summary>Rule-detected items a data-quality review may choose from.</summary>
    public JsonArray Candidates { get; }

    /// <summary>Member handles to the people they stand for. Never serialized.</summary>
    public IReadOnlyDictionary<string, AiAlias> Resolution { get; }

    /// <summary>Every id a placeholder may name: facts, labels, member handles and <c>ME</c>.</summary>
    public IReadOnlySet<string> KnownIds { get; }

    /// <summary>True when there is nothing to describe (no facts, lists or candidates): no model is called.</summary>
    public bool IsEmpty => Facts.Count == 0 && Lists.Count == 0 && Candidates.Count == 0;
}
