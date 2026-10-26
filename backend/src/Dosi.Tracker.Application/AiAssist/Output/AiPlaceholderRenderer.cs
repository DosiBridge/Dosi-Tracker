using System.Text.RegularExpressions;
using Dosi.Tracker.AiAssist.Context;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>
/// Turns a validated answer into what the reader sees, from the reader's own bundle: a fact becomes its display value,
/// a member handle the member's name, a project, app, task, title or note id its sanitized text, and <c>{ME}</c> "you"
/// ("You" at the start of a sentence or line). One pass: substituted text is never scanned again, so a name or label
/// that looks like a placeholder stays as it is. Unknown placeholders (the validator refuses them) are left unchanged.
/// </summary>
public static class AiPlaceholderRenderer
{
    public const string You = "you";

    /// <param name="text">The validated answer text.</param>
    /// <param name="bundle">The current caller's bundle (cached answers are rendered from the current bundle too). The
    /// reader is always <c>{ME}</c> (<see cref="AiPseudonymizer"/>), so they read "you" in every audience, never their
    /// own name.</param>
    public static string Render(string? text, AiContextBundle bundle)
    {
        if (string.IsNullOrEmpty(text))
        {
            return string.Empty;
        }

        return AiGroundingValidator.Placeholder.Replace(text, match => Substitute(match, text, bundle));
    }

    private static string Substitute(Match match, string text, AiContextBundle bundle)
    {
        var id = match.Groups[1].Value;

        if (id == AiPseudonymizer.SelfHandle)
        {
            return StartsSentence(text, match.Index) ? "You" : You;
        }

        if (bundle.Facts.TryGetValue(id, out var fact))
        {
            return fact.Display;
        }

        if (bundle.Resolution.TryGetValue(id, out var alias))
        {
            return alias.DisplayName;
        }

        return bundle.Labels.TryGetValue(id, out var label) ? label : match.Value;
    }

    /// <summary>True at the start of the text or of a line (after an optional "- " list marker), or after a
    /// sentence's final punctuation.</summary>
    private static bool StartsSentence(string text, int index)
    {
        var i = index - 1;
        while (i >= 0 && text[i] is ' ' or '\t')
        {
            i--;
        }

        if (i < 0 || text[i] is '\n' or '\r' or '.' or '!' or '?')
        {
            return true;
        }

        // "- {ME} …" at a line start (a list item).
        if (text[i] == '-')
        {
            var j = i - 1;
            while (j >= 0 && text[j] is ' ' or '\t')
            {
                j--;
            }

            return j < 0 || text[j] is '\n' or '\r';
        }

        return false;
    }
}
