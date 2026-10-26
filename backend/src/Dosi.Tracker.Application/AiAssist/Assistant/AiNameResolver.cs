using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Read;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Replaces a member's name in a question with their handle, from the roster the caller may see (spec 10.2). Matching
/// ignores case and diacritics and respects word boundaries; a token shorter than 3 characters is never a name. A
/// first-name-only token (no surname or full name given) is a name only when it is capitalized in the question, and
/// only when exactly one visible member has that first name: more than one gives a clarification instead of a guess.
/// A name the caller cannot see (not in the roster) is left exactly as typed, which is the point: the disclosure says
/// so, and the model never learns who it was.
/// </summary>
public static class AiNameResolver
{
    private static readonly Regex Token = new(@"[\p{L}][\p{L}\p{N}'-]*", RegexOptions.Compiled | RegexOptions.CultureInvariant);

    /// <summary>What <see cref="Resolve"/> made of a question.</summary>
    /// <param name="RewrittenQuestion">The question with every recognized name replaced by its handle.</param>
    /// <param name="AmbiguousNames">The names (as typed) that matched more than one visible member; empty when none
    /// did. When this is not empty, the caller should answer with a clarification instead of asking the model.</param>
    public sealed record Result(string RewrittenQuestion, IReadOnlyList<string> AmbiguousNames);

    public static Result Resolve(string question, IReadOnlyList<AiRosterEntry> roster, AiPseudonymizer pseudonymizer)
    {
        if (string.IsNullOrEmpty(question) || roster.Count == 0)
        {
            return new Result(question ?? string.Empty, Array.Empty<string>());
        }

        var tokens = Token.Matches(question).Cast<Match>().ToList();
        var replacements = new List<(int Start, int Length, string Text)>();
        var ambiguous = new List<string>();
        var used = new HashSet<int>();

        for (var i = 0; i < tokens.Count; i++)
        {
            if (used.Contains(i))
            {
                continue;
            }

            if (i + 1 < tokens.Count && !used.Contains(i + 1))
            {
                var fullKey = Normalize(tokens[i].Value + " " + tokens[i + 1].Value);
                var fullMatches = roster.Where(r => FullKey(r) == fullKey).ToList();
                if (fullMatches.Count == 1)
                {
                    var start = tokens[i].Index;
                    var length = tokens[i + 1].Index + tokens[i + 1].Length - start;
                    replacements.Add((start, length, HandleFor(fullMatches[0], pseudonymizer)));
                    used.Add(i);
                    used.Add(i + 1);
                    continue;
                }

                if (fullMatches.Count > 1)
                {
                    ambiguous.Add(tokens[i].Value + " " + tokens[i + 1].Value);
                    used.Add(i);
                    used.Add(i + 1);
                    continue;
                }
            }

            var token = tokens[i].Value;
            if (token.Length < 3)
            {
                continue;
            }

            var key = Normalize(token);
            var byUserName = roster.Where(r => UserKey(r) == key).ToList();
            var bySurname = roster.Where(r => SurnameKey(r) == key).ToList();
            var byFirstName = roster.Where(r => FirstKey(r) == key).ToList();

            if (byUserName.Count == 1 && bySurname.Count == 0 && byFirstName.Count == 0)
            {
                replacements.Add((tokens[i].Index, token.Length, HandleFor(byUserName[0], pseudonymizer)));
                continue;
            }

            if (bySurname.Count == 1 && byFirstName.Count == 0)
            {
                replacements.Add((tokens[i].Index, token.Length, HandleFor(bySurname[0], pseudonymizer)));
                continue;
            }

            if (byFirstName.Count == 0 || !char.IsUpper(token[0]))
            {
                // Not capitalized: a first name is only a name when the question capitalizes it.
                continue;
            }

            if (byFirstName.Count == 1)
            {
                replacements.Add((tokens[i].Index, token.Length, HandleFor(byFirstName[0], pseudonymizer)));
            }
            else
            {
                ambiguous.Add(token);
            }
        }

        if (ambiguous.Count > 0)
        {
            return new Result(question, ambiguous);
        }

        if (replacements.Count == 0)
        {
            return new Result(question, Array.Empty<string>());
        }

        var builder = new StringBuilder(question);
        foreach (var (start, length, text) in replacements.OrderByDescending(r => r.Start))
        {
            builder.Remove(start, length).Insert(start, text);
        }

        return new Result(builder.ToString(), Array.Empty<string>());
    }

    private static string HandleFor(AiRosterEntry member, AiPseudonymizer pseudonymizer) =>
        member.UserId == pseudonymizer.CallerId ? AiPseudonymizer.SelfHandle : pseudonymizer.GetOrAdd(member.UserId);

    private static string? FullKey(AiRosterEntry member)
    {
        var full = string.Join(" ", new[] { member.Name, member.Surname }.Where(p => !string.IsNullOrWhiteSpace(p)));
        return full.Length == 0 ? null : Normalize(full);
    }

    private static string? FirstKey(AiRosterEntry member) =>
        string.IsNullOrWhiteSpace(member.Name) || member.Name.Trim().Length < 3 ? null : Normalize(member.Name);

    private static string? SurnameKey(AiRosterEntry member) =>
        string.IsNullOrWhiteSpace(member.Surname) || member.Surname.Trim().Length < 3 ? null : Normalize(member.Surname);

    private static string? UserKey(AiRosterEntry member) =>
        string.IsNullOrWhiteSpace(member.UserName) || member.UserName.Trim().Length < 3 ? null : Normalize(member.UserName);

    /// <summary>Case- and diacritic-insensitive: NFD, combining marks stripped, lower-invariant.</summary>
    private static string Normalize(string text)
    {
        var decomposed = text.Trim().Normalize(NormalizationForm.FormD);
        var builder = new StringBuilder(decomposed.Length);
        foreach (var c in decomposed)
        {
            if (CharUnicodeInfo.GetUnicodeCategory(c) != UnicodeCategory.NonSpacingMark)
            {
                builder.Append(c);
            }
        }

        return builder.ToString().ToLowerInvariant();
    }
}
