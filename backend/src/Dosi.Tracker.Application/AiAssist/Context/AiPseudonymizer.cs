using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>A member and the tracked minutes that order the handles of a card.</summary>
public readonly record struct AiMemberWeight(Guid UserId, double Minutes);

/// <summary>
/// Gives people handles so no name, user name or email reaches the model: the reader is always <see cref="SelfHandle"/>,
/// everyone else <c>M1</c>, <c>M2</c> …. A handle, once given, never changes.
/// <para>
/// Cards (<see cref="ForCard"/>) number their members up front, by tracked minutes (most first) then user id, so the
/// same data always gets the same handles. A conversation (<see cref="ForConversation"/>) keeps the handles it has
/// already given and numbers newcomers in order of first appearance.
/// </para>
/// </summary>
public sealed class AiPseudonymizer
{
    public const string SelfHandle = "ME";
    public const string MemberPrefix = "M";

    private readonly Dictionary<Guid, string> _handles = new();
    private int _lastNumber;

    /// <summary>A pseudonymizer that numbers members in order of first appearance.</summary>
    public AiPseudonymizer(Guid callerId)
    {
        CallerId = callerId;
        _handles[callerId] = SelfHandle;
    }

    public Guid CallerId { get; }

    /// <summary>Every handle given so far (the caller's included).</summary>
    public IReadOnlyDictionary<Guid, string> Handles => _handles;

    /// <summary>
    /// Handles for a card: the <paramref name="max"/> members with the most tracked minutes (ties by user id) get
    /// <c>M1</c>…<c>Mn</c> in that order; the caller is <see cref="SelfHandle"/> and takes no number. Members beyond
    /// <paramref name="max"/> get no handle (<see cref="TryGetHandle"/> is false), so the card folds them into "others".
    /// </summary>
    public static AiPseudonymizer ForCard(Guid callerId, IEnumerable<AiMemberWeight> members, int max = AiAssistConsts.MaxBundleMembers)
    {
        var pseudonymizer = new AiPseudonymizer(callerId);
        var ranked = members
            .Where(m => m.UserId != callerId)
            .GroupBy(m => m.UserId)
            .Select(g => new AiMemberWeight(g.Key, g.Sum(m => m.Minutes)))
            .OrderByDescending(m => m.Minutes)
            .ThenBy(m => m.UserId)
            .Take(Math.Max(0, max));

        foreach (var member in ranked)
        {
            pseudonymizer.GetOrAdd(member.UserId);
        }

        return pseudonymizer;
    }

    /// <summary>A conversation's pseudonymizer: the handles it gave in earlier turns stay, newcomers get the next
    /// number. Handles that are malformed, or name the caller as anyone but <see cref="SelfHandle"/>, are ignored.</summary>
    public static AiPseudonymizer ForConversation(Guid callerId, IReadOnlyDictionary<Guid, string>? existing)
    {
        var pseudonymizer = new AiPseudonymizer(callerId);
        foreach (var (userId, handle) in existing ?? new Dictionary<Guid, string>())
        {
            if (userId == callerId || !TryParseNumber(handle, out var number) ||
                pseudonymizer._handles.ContainsValue(handle))
            {
                continue;
            }

            pseudonymizer._handles[userId] = handle;
            pseudonymizer._lastNumber = Math.Max(pseudonymizer._lastNumber, number);
        }

        return pseudonymizer;
    }

    /// <summary>The member's handle, giving the next one on first appearance.</summary>
    public string GetOrAdd(Guid userId)
    {
        if (_handles.TryGetValue(userId, out var handle))
        {
            return handle;
        }

        handle = MemberPrefix + (++_lastNumber).ToString(CultureInfo.InvariantCulture);
        _handles[userId] = handle;
        return handle;
    }

    /// <summary>The member's handle, if they have one.</summary>
    public bool TryGetHandle(Guid userId, out string handle)
    {
        return _handles.TryGetValue(userId, out handle!);
    }

    /// <summary>The member a handle stands for, if it was given.</summary>
    public bool TryResolve(string handle, out Guid userId)
    {
        foreach (var (id, given) in _handles)
        {
            if (string.Equals(given, handle, StringComparison.Ordinal))
            {
                userId = id;
                return true;
            }
        }

        userId = default;
        return false;
    }

    private static bool TryParseNumber(string? handle, out int number)
    {
        number = 0;
        return handle is { Length: > 1 } &&
               handle.StartsWith(MemberPrefix, StringComparison.Ordinal) &&
               int.TryParse(handle.AsSpan(1), NumberStyles.None, CultureInfo.InvariantCulture, out number) &&
               number > 0 &&
               handle == MemberPrefix + number.ToString(CultureInfo.InvariantCulture);
    }
}
