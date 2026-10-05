using System;
using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The one canonical display name of an application (shared contract §8). Agents report apps the way their
/// platform sees them — the Windows agent sends the executable ("chrome.exe", "Code.exe"), the macOS agent the
/// app's localized name ("Google Chrome", "Code", "zoom.us") — and reports group every spelling of an app under
/// its canonical name. The web's <c>canonicalAppName</c> (frontend/src/lib/apps.ts) applies the same mapping, so
/// the alias table here mirrors that one: change one, change the other in the same release.
/// </summary>
public static class AppNames
{
    private const string ExeSuffix = ".exe";

    /// <summary>
    /// Case-insensitive aliases, keyed by the lower-cased name with ".exe" stripped. The second group maps every
    /// canonical name to itself, so canonicalising a name that is already canonical is a no-op ("Windows app"
    /// must not become "Windows App").
    /// </summary>
    private static readonly Dictionary<string, string> Aliases = new(StringComparer.Ordinal)
    {
        ["chrome"] = "Google Chrome",
        ["google chrome"] = "Google Chrome",
        ["msedge"] = "Microsoft Edge",
        ["code"] = "Visual Studio Code",
        ["firefox"] = "Firefox",
        ["slack"] = "Slack",
        ["teams"] = "Microsoft Teams",
        ["ms-teams"] = "Microsoft Teams",
        ["outlook"] = "Outlook",
        ["winword"] = "Word",
        ["excel"] = "Excel",
        ["powerpnt"] = "PowerPoint",
        ["explorer"] = "File Explorer",
        ["finder"] = "Finder",
        ["windowsterminal"] = "Windows Terminal",
        ["terminal"] = "Terminal",
        ["iterm2"] = "iTerm2",
        ["devenv"] = "Visual Studio",
        ["idea64"] = "IntelliJ IDEA",
        ["rider64"] = "Rider",
        ["figma"] = "Figma",
        ["zoom"] = "Zoom",
        ["zoom.us"] = "Zoom",
        ["notepad"] = "Notepad",
        ["applicationframehost"] = "Windows app",
        ["lockapp"] = "Lock screen",

        ["microsoft edge"] = "Microsoft Edge",
        ["visual studio code"] = "Visual Studio Code",
        ["microsoft teams"] = "Microsoft Teams",
        ["word"] = "Word",
        ["powerpoint"] = "PowerPoint",
        ["file explorer"] = "File Explorer",
        ["windows terminal"] = "Windows Terminal",
        ["visual studio"] = "Visual Studio",
        ["intellij idea"] = "IntelliJ IDEA",
        ["rider"] = "Rider",
        ["windows app"] = "Windows app",
        ["lock screen"] = "Lock screen",
        ["unknown app"] = "Unknown app",
        ["other apps"] = "Other apps"
    };

    /// <summary>Runs of whitespace (and byte-order marks, which JavaScript's trim also removes).</summary>
    private static readonly Regex Whitespace = new(@"[\s﻿]+", RegexOptions.Compiled | RegexOptions.CultureInvariant);

    /// <summary>
    /// The canonical name of an app as an agent reported it: whitespace collapsed, every trailing ".exe" stripped,
    /// the alias table applied case-insensitively, otherwise the first letter of each word upper-cased (the rest is
    /// kept: "myApp tool.exe" → "MyApp Tool"). Empty when there is no name at all. Idempotent.
    /// </summary>
    public static string Canonical(string? raw)
    {
        if (raw == null)
        {
            return string.Empty;
        }

        var stem = Tidy(raw);
        while (stem.EndsWith(ExeSuffix, StringComparison.OrdinalIgnoreCase))
        {
            stem = Tidy(stem[..^ExeSuffix.Length]);
        }

        if (stem.Length == 0)
        {
            return string.Empty;
        }

        return Aliases.TryGetValue(stem.ToLowerInvariant(), out var alias) ? alias : TitleCase(stem);
    }

    /// <summary>The key app rows are grouped by: the canonical name, lower-cased, so "MyTool.exe" and "mytool.exe"
    /// (Windows names ignore case) are one app. Empty when there is no name.</summary>
    public static string GroupKey(string? raw)
    {
        return Canonical(raw).ToLowerInvariant();
    }

    private static string Tidy(string value)
    {
        return Whitespace.Replace(value, " ").Trim();
    }

    private static string TitleCase(string stem)
    {
        var words = stem.Split(' ');
        for (var i = 0; i < words.Length; i++)
        {
            var word = words[i];
            if (word.Length > 0)
            {
                words[i] = char.ToUpperInvariant(word[0]) + word[1..];
            }
        }

        return string.Join(' ', words);
    }
}
