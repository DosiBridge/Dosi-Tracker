using System;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>
/// Makes text that people or software wrote (project, app and task names, window titles, notes, goals, help snippets)
/// safe to quote to a model: it can no longer forge a delimiter or a placeholder, impersonate a chat role, carry a
/// link, a path, an address, a long number or a credential, or tell the model what to do.
/// <para>
/// <see cref="Sanitize(string?, int, out bool)"/> applies, in this order: NFC normalization; removal of control and
/// format characters (zero-width and bidi marks included; line breaks are kept until the end); angle and curly
/// brackets swapped for look-alikes that cannot open a tag or a placeholder; removal of triple backticks; role
/// prefixes at a line start neutralized; URLs cut to their host and file paths to their file name; email addresses,
/// credential-shaped tokens and runs of 6 or more digits redacted; instruction-shaped phrases removed; whitespace
/// collapsed and the text cut to its limit. The steps are repeated until the text no longer changes, so the function
/// is idempotent.
/// </para>
/// </summary>
public static class UntrustedText
{
    public const string EmailMarker = "[email]";
    public const string NumberMarker = "[number]";
    public const string SecretMarker = "[redacted]";
    public const string RemovedMarker = "[text removed]";

    private const RegexOptions Options = RegexOptions.Compiled | RegexOptions.CultureInvariant;
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(1);

    // Rounds after the first that may still change the text (each removes something); bounds hostile inputs.
    private const int MaxRounds = 4;

    private static readonly Regex TripleBackticks = new("`{3,}", Options, Timeout);

    private static readonly Regex RolePrefix = new(
        @"^[ \t]*(system|assistant|developer|user|tool)[ \t]*:", Options | RegexOptions.IgnoreCase | RegexOptions.Multiline, Timeout);

    private static readonly Regex Url = new(
        @"\b(?:(?<scheme>https?|ftp|file)://|(?=www\.))(?<rest>[^\s‹›()\[\]""'`]+)",
        Options | RegexOptions.IgnoreCase, Timeout);

    // A drive or UNC path: everything up to its last backslash on the line goes, so "C:\Users\Jane Doe\a b.xlsx" keeps
    // only "a b.xlsx" (folder names, user names included, never reach the model).
    private static readonly Regex WindowsPath = new(@"(?:\b[A-Za-z]:\\|\\\\)(?:[^\\\n]*\\)*", Options, Timeout);

    // What is left of a backslash path ("Users\Jane\notes.txt").
    private static readonly Regex RelativeWindowsPath = new(@"(?:[^\s\\]+\\)+", Options, Timeout);

    // An absolute or home-relative POSIX path of at least one folder ("/home/jane/notes.txt", "~/a/b.txt"); a lone
    // slash between words ("and/or", "Inbox / Work") is not a path.
    private static readonly Regex PosixPath = new(@"(?<![\w.:/~])(?:~|\.{1,2})?/(?:[^/\s]+/)+", Options, Timeout);

    private static readonly Regex Email = new(
        @"[\p{L}\p{N}._%+-]+@[\p{L}\p{N}-]+(?:\.[\p{L}\p{N}-]+)*\.\p{L}{2,}", Options, Timeout);

    private static readonly Regex Secret = new(
        string.Join('|',
            @"\b(?:sk|pk)-[A-Za-z0-9_-]{16,}",
            @"\bghp_\w{20,}",
            @"\bxox[abp]-[\w-]{10,}",
            @"\bAKIA[0-9A-Z]{16}\b",
            @"\beyJ[\w-]{10,}\.[\w-]{10,}\.[\w-]+",
            @"\b[0-9A-Fa-f]{32,}\b",
            // Base64 of 40+ characters holding both a letter and a digit (a long plain word is not a credential).
            @"(?<![A-Za-z0-9+/])(?=[A-Za-z0-9+/]*[0-9])(?=[A-Za-z0-9+/]*[A-Za-z])[A-Za-z0-9+/]{40,}={0,2}"),
        Options, Timeout);

    private static readonly Regex DigitRun = new(@"\p{Nd}{6,}", Options, Timeout);

    private static readonly Regex Injection = new(
        @"\b(?:ignore|disregard|forget)\b.{0,40}\b(?:instructions?|rules?|prompt|above)\b|\b(?:system prompt|you are now|act as|jailbreak)\b",
        Options | RegexOptions.IgnoreCase | RegexOptions.Singleline, Timeout);

    private static readonly Regex Whitespace = new(@"\s+", Options, Timeout);

    /// <summary>The text made safe to quote, at most <paramref name="max"/> characters (empty for null or blank).</summary>
    public static string Sanitize(string? raw, int max) => Sanitize(raw, max, out _);

    /// <param name="raw">What people or software wrote.</param>
    /// <param name="max">The longest result; longer text is cut and ends with "…".</param>
    /// <param name="injectionRemoved">True when an instruction-shaped phrase was removed (the bundle then carries a
    /// caveat that some text was omitted).</param>
    public static string Sanitize(string? raw, int max, out bool injectionRemoved)
    {
        injectionRemoved = false;
        if (string.IsNullOrWhiteSpace(raw) || max <= 0)
        {
            return string.Empty;
        }

        var text = raw;
        try
        {
            for (var round = 0; round <= MaxRounds; round++)
            {
                var next = SanitizeOnce(text, max, out var removed);
                injectionRemoved |= removed;
                if (next == text)
                {
                    break;
                }

                text = next;
            }
        }
        catch (RegexMatchTimeoutException)
        {
            // Text too hostile to check in time is not quoted at all.
            return string.Empty;
        }

        return text;
    }

    /// <summary>A question someone typed to Ask AI: only email addresses and credential-shaped tokens are redacted;
    /// everything else is kept as typed (the question is the user's own words, delimited as such).</summary>
    public static string SanitizeQuestion(string? raw)
    {
        if (string.IsNullOrEmpty(raw))
        {
            return string.Empty;
        }

        try
        {
            return Secret.Replace(Email.Replace(raw, EmailMarker), SecretMarker);
        }
        catch (RegexMatchTimeoutException)
        {
            return string.Empty;
        }
    }

    private static string SanitizeOnce(string text, int max, out bool injectionRemoved)
    {
        // 1. One spelling per character (a decomposed "é" and a composed one compare equal afterwards).
        text = DropLoneSurrogates(text).Normalize(NormalizationForm.FormC);

        // 2. Controls and invisible format characters go; line breaks stay (for step 5), other whitespace controls
        // become spaces.
        text = StripControls(text);

        // 3. Nothing can open or close a tag or a placeholder.
        text = SwapBrackets(text);

        // 4. No code fences.
        text = TripleBackticks.Replace(text, string.Empty);

        // 5. "system:" at a line start no longer reads as a chat turn.
        text = RolePrefix.Replace(text, "$1 -");

        // 6. Links keep their host; paths their file name.
        text = Url.Replace(text, HostOf);
        text = WindowsPath.Replace(text, string.Empty);
        text = RelativeWindowsPath.Replace(text, string.Empty);
        text = PosixPath.Replace(text, string.Empty);

        // 7 and 8. Addresses, credentials, then long numbers (credentials first, so a key holding a run of digits is
        // redacted whole rather than split around a number marker).
        text = Email.Replace(text, EmailMarker);
        text = Secret.Replace(text, SecretMarker);
        text = DigitRun.Replace(text, NumberMarker);

        // 9. Instruction-shaped phrases.
        var removed = false;
        text = Injection.Replace(text, _ =>
        {
            removed = true;
            return RemovedMarker;
        });
        injectionRemoved = removed;

        // 10. One line, within the limit.
        text = Whitespace.Replace(text, " ").Trim();
        return Truncate(text, max);
    }

    /// <summary>The text cut to <paramref name="max"/> characters, ending with "…" when cut; never splits a
    /// surrogate pair.</summary>
    internal static string Truncate(string text, int max)
    {
        if (text.Length <= max)
        {
            return text;
        }

        if (max == 1)
        {
            return "…";
        }

        var cut = max - 1;
        if (char.IsHighSurrogate(text[cut - 1]))
        {
            cut--;
        }

        return text[..cut].TrimEnd() + "…";
    }

    private static string HostOf(Match match)
    {
        var scheme = match.Groups["scheme"].Value;
        var rest = match.Groups["rest"].Value;

        if (scheme.Equals("file", StringComparison.OrdinalIgnoreCase))
        {
            // file:///C:/Users/jane/a.txt: a path, so only its file name is kept.
            var lastSlash = rest.LastIndexOfAny(new[] { '/', '\\' });
            return lastSlash < 0 ? rest : rest[(lastSlash + 1)..];
        }

        var end = rest.IndexOfAny(new[] { '/', '?', '#', '\\' });
        var authority = end < 0 ? rest : rest[..end];
        var at = authority.LastIndexOf('@');
        if (at >= 0)
        {
            authority = authority[(at + 1)..];
        }

        var colon = authority.IndexOf(':');
        return colon < 0 ? authority : authority[..colon];
    }

    private static string DropLoneSurrogates(string text)
    {
        StringBuilder? builder = null;
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            var paired = char.IsHighSurrogate(c)
                ? i + 1 < text.Length && char.IsLowSurrogate(text[i + 1])
                : !char.IsLowSurrogate(c) || (i > 0 && char.IsHighSurrogate(text[i - 1]));

            if (!paired)
            {
                builder ??= new StringBuilder(text, 0, i, text.Length);
                continue;
            }

            builder?.Append(c);
        }

        return builder?.ToString() ?? text;
    }

    private static string StripControls(string text)
    {
        var builder = new StringBuilder(text.Length);
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            switch (c)
            {
                case '\r':
                    builder.Append('\n');
                    if (i + 1 < text.Length && text[i + 1] == '\n')
                    {
                        i++;
                    }

                    continue;
                case '\n' or '\u0085' or '\u2028' or '\u2029':
                    builder.Append('\n');
                    continue;
                case '\t' or '\v' or '\f':
                    builder.Append(' ');
                    continue;
            }

            var category = CharUnicodeInfo.GetUnicodeCategory(c);
            if (category is UnicodeCategory.Control or UnicodeCategory.Format)
            {
                continue;
            }

            builder.Append(c);
        }

        return builder.ToString();
    }

    private static string SwapBrackets(string text)
    {
        var builder = new StringBuilder(text.Length);
        foreach (var c in text)
        {
            builder.Append(c switch
            {
                '<' or '＜' or '﹤' => '‹',
                '>' or '＞' or '﹥' => '›',
                '{' or '｛' or '﹛' => '(',
                '}' or '｝' or '﹜' => ')',
                _ => c
            });
        }

        return builder.ToString();
    }
}
