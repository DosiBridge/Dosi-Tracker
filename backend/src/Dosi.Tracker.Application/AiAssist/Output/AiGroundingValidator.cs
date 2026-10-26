using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.RegularExpressions;
using Dosi.Tracker.AiAssist.Context;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>
/// The output rules every AI answer is checked against before anyone sees it. Text must cite the bundle through
/// placeholders and say nothing the data does not; a violation earns one repair turn, then the deterministic fallback.
/// <list type="number">
/// <item>Placeholders match <c>{ME}</c> or <c>{M1}</c>, <c>{P1}</c>, <c>{A1}</c>, <c>{T1}</c>, <c>{W1}</c>, <c>{N1}</c>,
/// <c>{F1}</c> (up to three digits) and name an id of the bundle; fact references name facts.</item>
/// <item>Grounded and chat text, placeholders removed, holds no digit, number word, full weekday name or full month
/// name ("May" excepted: it is also a word).</item>
/// <item>Creative text may hold a run of digits only when an input it was made from holds it, and names no person.</item>
/// <item>No judging terms (<see cref="AiBannedTerms"/>), no links, no markup; route ids are allowlisted and offered.</item>
/// <item>Chat answers are at most <see cref="AiAssistConsts.MaxAnswerLength"/> characters and have no heading lines.</item>
/// </list>
/// Messages never quote the output, so they are safe to log and to send back in the repair turn.
/// </summary>
public static class AiGroundingValidator
{
    private const RegexOptions Options = RegexOptions.Compiled | RegexOptions.CultureInvariant;
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(1);

    /// <summary>A valid placeholder; group 1 is its id.</summary>
    public static readonly Regex Placeholder = new(@"\{(ME|[MPATWNF][0-9]{1,3})\}", Options, Timeout);

    private static readonly Regex Braced = new(@"\{[^{}\n]*\}", Options, Timeout);
    private static readonly Regex Digit = new(@"\p{Nd}", Options, Timeout);
    private static readonly Regex DigitRun = new(@"\p{Nd}+", Options, Timeout);

    private static readonly Regex NumberWord = new(
        @"\b(?:two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|twenty|thirty|forty|fifty|hundred|thousand|dozen|half|twice|double|triple|percent)\b",
        Options | RegexOptions.IgnoreCase, Timeout);

    private static readonly Regex Weekday = new(
        @"\b(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b", Options | RegexOptions.IgnoreCase, Timeout);

    private static readonly Regex Month = new(
        @"\b(?:january|february|march|april|june|july|august|september|october|november|december)\b",
        Options | RegexOptions.IgnoreCase, Timeout);

    private static readonly Regex Link = new(@"https?://|\bwww\.", Options | RegexOptions.IgnoreCase, Timeout);

    private static readonly Regex Markup = new(@"<|^[ \t]*#|\*\*|`", Options | RegexOptions.Multiline, Timeout);

    private static readonly Regex HeadingLine = new(@"^[ \t]*(?:#.*|[=-]{3,}[ \t]*)$", Options | RegexOptions.Multiline, Timeout);

    /// <summary>Checks one text field of an answer.</summary>
    /// <param name="path">Where the text is in the answer (<c>headline</c>, <c>insights[0].text</c>).</param>
    /// <param name="text">The text; null or empty has nothing to check (required fields are checked by
    /// <see cref="ValidateRequired"/>).</param>
    /// <param name="profile">Which rules apply.</param>
    /// <param name="knownIds">The ids placeholders may name (<see cref="AiContextBundle.KnownIds"/>).</param>
    /// <param name="quotedInputs">For <see cref="AiValidationProfile.Creative"/>: the texts the answer was made from,
    /// whose digits it may repeat.</param>
    public static IReadOnlyList<AiViolation> ValidateText(
        string path,
        string? text,
        AiValidationProfile profile,
        IReadOnlySet<string> knownIds,
        IReadOnlyCollection<string>? quotedInputs = null)
    {
        var violations = new List<AiViolation>();
        if (string.IsNullOrEmpty(text))
        {
            return violations;
        }

        try
        {
            CheckPlaceholders(path, text, profile, knownIds, violations);

            var withoutPlaceholders = Placeholder.Replace(text, " ");
            if (profile == AiValidationProfile.Creative)
            {
                CheckQuotedDigits(path, withoutPlaceholders, quotedInputs, violations);
            }
            else
            {
                CheckGrounded(path, withoutPlaceholders, violations);
            }

            if (AiBannedTerms.Contains(withoutPlaceholders))
            {
                violations.Add(new AiViolation(path, "uses judging language about people; describe the data neutrally"));
            }

            if (Link.IsMatch(text))
            {
                violations.Add(new AiViolation(path, "contains a link; links may only use the offered route ids"));
            }

            if (Markup.IsMatch(text))
            {
                violations.Add(new AiViolation(path, "contains markup; write plain text only"));
            }

            if (profile == AiValidationProfile.Chat)
            {
                CheckChat(path, text, violations);
            }
        }
        catch (RegexMatchTimeoutException)
        {
            violations.Add(new AiViolation(path, "could not be checked"));
        }

        return violations;
    }

    /// <summary>Checks that every fact reference (<c>F1</c> or <c>{F1}</c>) names a fact the bundle holds.</summary>
    public static IReadOnlyList<AiViolation> ValidateFactRefs(string path, IEnumerable<string?>? factRefs, IReadOnlySet<string> knownIds)
    {
        var violations = new List<AiViolation>();
        var index = 0;
        foreach (var factRef in factRefs ?? Array.Empty<string?>())
        {
            var id = NormalizeRef(factRef);
            if (id == null || !id.StartsWith(AiBundleWriter.FactPrefix, StringComparison.Ordinal) || !knownIds.Contains(id))
            {
                violations.Add(new AiViolation($"{path}[{index}]", "names no fact of the data"));
            }

            index++;
        }

        return violations;
    }

    /// <summary>A route id must be allowlisted (<see cref="AiRouteIds"/>) and offered in the data's routes; empty is
    /// accepted when <paramref name="allowEmpty"/> (the schema's "no link").</summary>
    public static AiViolation? ValidateRouteId(string path, string? routeId, IReadOnlyCollection<string> offeredRoutes, bool allowEmpty = true)
    {
        if (string.IsNullOrEmpty(routeId))
        {
            return allowEmpty ? null : new AiViolation(path, "needs one of the offered route ids");
        }

        return AiRouteIds.IsAllowed(routeId) && offeredRoutes.Contains(routeId)
            ? null
            : new AiViolation(path, "is not one of the offered route ids");
    }

    /// <summary>The value must be one of <paramref name="allowed"/> (an enum, a candidate id, an allowed action);
    /// empty is accepted when <paramref name="allowEmpty"/>.</summary>
    public static AiViolation? ValidateChoice(string path, string? value, IReadOnlyCollection<string> allowed, bool allowEmpty = false)
    {
        if (string.IsNullOrEmpty(value))
        {
            return allowEmpty ? null : new AiViolation(path, "is required");
        }

        return allowed.Contains(value) ? null : new AiViolation(path, "is not one of the allowed values");
    }

    /// <summary>A required text must be present and not blank.</summary>
    public static AiViolation? ValidateRequired(string path, string? value)
    {
        return string.IsNullOrWhiteSpace(value) ? new AiViolation(path, "is required") : null;
    }

    /// <summary>The ids the text's placeholders name, in order of first use.</summary>
    public static IReadOnlyList<string> PlaceholderIds(string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return Array.Empty<string>();
        }

        return Placeholder.Matches(text).Select(m => m.Groups[1].Value).Distinct(StringComparer.Ordinal).ToList();
    }

    /// <summary>The text cut to <paramref name="max"/> characters (ending with "…" when cut), never inside a
    /// placeholder; trimmed; empty for null.</summary>
    public static string Cap(string? text, int max)
    {
        var trimmed = (text ?? string.Empty).Trim();
        if (trimmed.Length <= max)
        {
            return trimmed;
        }

        if (max <= 1)
        {
            return max == 1 ? "…" : string.Empty;
        }

        var cut = max - 1;
        // Never leave half a placeholder: an opening brace without its closing one before the cut goes too.
        var open = trimmed.LastIndexOf('{', cut - 1);
        if (open >= 0)
        {
            var close = trimmed.IndexOf('}', open);
            if (close < 0 || close >= cut)
            {
                cut = open;
            }
        }

        if (cut > 0 && char.IsHighSurrogate(trimmed[cut - 1]))
        {
            cut--;
        }

        return trimmed[..cut].TrimEnd() + "…";
    }

    /// <summary>The first <paramref name="max"/> items (extra ones are dropped); empty for null.</summary>
    public static List<T> CapList<T>(IEnumerable<T>? items, int max)
    {
        return items == null ? new List<T>() : items.Take(Math.Max(0, max)).ToList();
    }

    private static void CheckPlaceholders(
        string path, string text, AiValidationProfile profile, IReadOnlySet<string> knownIds, List<AiViolation> violations)
    {
        foreach (Match braced in Braced.Matches(text))
        {
            var placeholder = Placeholder.Match(braced.Value);
            if (!placeholder.Success || placeholder.Length != braced.Length)
            {
                violations.Add(new AiViolation(path, "has a malformed placeholder"));
                continue;
            }

            var id = placeholder.Groups[1].Value;
            var isPerson = id == AiPseudonymizer.SelfHandle || id.StartsWith(AiPseudonymizer.MemberPrefix, StringComparison.Ordinal);
            if (profile == AiValidationProfile.Creative && isPerson)
            {
                violations.Add(new AiViolation(path, "names a person; leave people out"));
            }
            else if (id != AiPseudonymizer.SelfHandle && !knownIds.Contains(id))
            {
                violations.Add(new AiViolation(path, $"uses the placeholder {{{id}}}, which the data does not hold"));
            }
        }

        var rest = Braced.Replace(text, string.Empty);
        if (rest.Contains('{') || rest.Contains('}'))
        {
            violations.Add(new AiViolation(path, "has a stray brace"));
        }
    }

    private static void CheckGrounded(string path, string text, List<AiViolation> violations)
    {
        if (Digit.IsMatch(text))
        {
            violations.Add(new AiViolation(path, "contains a digit; write every value only as its fact placeholder"));
        }

        if (NumberWord.IsMatch(text))
        {
            violations.Add(new AiViolation(path, "contains a number word; write every value only as its fact placeholder"));
        }

        if (Weekday.IsMatch(text))
        {
            violations.Add(new AiViolation(path, "names a weekday; write dates only as their fact placeholder"));
        }

        if (Month.IsMatch(text))
        {
            violations.Add(new AiViolation(path, "names a month; write dates only as their fact placeholder"));
        }
    }

    private static void CheckQuotedDigits(string path, string text, IReadOnlyCollection<string>? quotedInputs, List<AiViolation> violations)
    {
        var inputs = quotedInputs ?? Array.Empty<string>();
        foreach (Match run in DigitRun.Matches(text))
        {
            if (!inputs.Any(input => input.Contains(run.Value, StringComparison.Ordinal)))
            {
                violations.Add(new AiViolation(path, "contains a number that is not in the inputs"));
                return;
            }
        }
    }

    private static void CheckChat(string path, string text, List<AiViolation> violations)
    {
        if (text.Length > AiAssistConsts.MaxAnswerLength)
        {
            violations.Add(new AiViolation(path, $"is longer than {AiAssistConsts.MaxAnswerLength} characters"));
        }

        if (HeadingLine.IsMatch(text))
        {
            violations.Add(new AiViolation(path, "has a heading line; write plain sentences and \"- \" list lines only"));
        }
    }

    private static string? NormalizeRef(string? factRef)
    {
        var trimmed = factRef?.Trim();
        if (string.IsNullOrEmpty(trimmed))
        {
            return null;
        }

        if (trimmed.Length > 2 && trimmed[0] == '{' && trimmed[^1] == '}')
        {
            trimmed = trimmed[1..^1];
        }

        return trimmed;
    }
}
