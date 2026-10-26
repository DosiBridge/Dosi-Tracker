using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Reporting;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>What a block's stored window list says, for AI: apps with their seconds, and titles only when asked for.</summary>
public sealed record AiWindowUsage(IReadOnlyList<AiAppSeconds> Apps, IReadOnlyList<AiWindowTitle> Titles)
{
    public static readonly AiWindowUsage Empty = new(Array.Empty<AiAppSeconds>(), Array.Empty<AiWindowTitle>());
}

/// <summary>
/// Reads a block's stored <c>ActiveWindowsJson</c> (the upload's <see cref="WindowInfoDto"/> list) without ever
/// failing: malformed or oversized JSON reads as nothing. Apps are grouped by their canonical name
/// (<see cref="AppNames.Canonical"/>), most seconds first; window titles are kept only when asked for.
/// </summary>
public static class AiActivityParsing
{
    /// <summary>Longer stored values are not read (the upload endpoint never stores one).</summary>
    public const int MaxJsonLength = 256 * 1024;

    private static readonly JsonSerializerOptions Options = new() { PropertyNameCaseInsensitive = true, MaxDepth = 8 };

    public static AiWindowUsage Parse(string? activeWindowsJson, bool includeTitles)
    {
        if (string.IsNullOrWhiteSpace(activeWindowsJson) || activeWindowsJson.Length > MaxJsonLength)
        {
            return AiWindowUsage.Empty;
        }

        List<WindowInfoDto?>? windows;
        try
        {
            windows = JsonSerializer.Deserialize<List<WindowInfoDto?>>(activeWindowsJson, Options);
        }
        catch (JsonException)
        {
            return AiWindowUsage.Empty;
        }
        catch (NotSupportedException)
        {
            return AiWindowUsage.Empty;
        }

        if (windows == null)
        {
            return AiWindowUsage.Empty;
        }

        var apps = new Dictionary<string, (string App, int? Seconds, int Order)>(StringComparer.Ordinal);
        var titles = new List<AiWindowTitle>();
        foreach (var window in windows)
        {
            var app = AppNames.Canonical(window?.AppName);
            if (app.Length == 0)
            {
                continue;
            }

            var seconds = window!.Seconds is >= 0 ? window.Seconds : null;
            var key = app.ToLowerInvariant();
            apps[key] = apps.TryGetValue(key, out var known)
                ? (known.App, Add(known.Seconds, seconds), known.Order)
                : (app, seconds, apps.Count);

            var title = window.WindowTitle?.Trim();
            if (includeTitles && !string.IsNullOrEmpty(title))
            {
                titles.Add(new AiWindowTitle(app, title, seconds));
            }
        }

        return new AiWindowUsage(
            apps.Values
                .OrderByDescending(a => a.Seconds ?? -1)
                .ThenBy(a => a.Order)
                .Select(a => new AiAppSeconds(a.App, a.Seconds))
                .ToList(),
            titles
                .OrderByDescending(t => t.Seconds ?? -1)
                .ToList());
    }

    private static int? Add(int? a, int? b)
    {
        if (a == null && b == null)
        {
            return null;
        }

        var sum = (long)(a ?? 0) + (b ?? 0);
        return sum > int.MaxValue ? int.MaxValue : (int)sum;
    }
}
