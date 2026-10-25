using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Formats activity records into RFC 4180 compliant CSV, strictly neutralizing
/// spreadsheet formula injection (DDE/CSV injection) and preserving unavailable metrics as blank.
/// </summary>
public static class CsvExportHelper
{
    private static readonly char[] FormulaTriggers = { '=', '+', '-', '@', '\t', '\r', '|' };

    public const string CsvHeader = "ActivityId,UserId,ProjectId,ProjectTitle,StartedAtUtc,EndedAtUtc,DurationMinutes,DurationSeconds,Description,Productivity,IdleSeconds,MouseClicks,KeyboardHits";

    /// <summary>
    /// Escapes a cell according to RFC 4180 rules and neutralizes potential spreadsheet formula injection.
    /// </summary>
    public static string EscapeCsvCell(string? value)
    {
        if (string.IsNullOrEmpty(value))
        {
            return string.Empty;
        }

        var cell = value;

        // Check if value or trimmed value starts with any formula trigger
        var trimmed = cell.TrimStart(' ', '\t', '\r', '\n', '\v', '\f');
        if (trimmed.Length > 0 && FormulaTriggers.Contains(trimmed[0]))
        {
            // Prepend single-quote to instruct spreadsheet engines (Excel, Sheets, Calc) to treat the cell as text
            cell = "'" + cell;
        }

        // Enclose in double quotes if it contains comma, quote, newline, or starts with single-quote
        bool needsQuotes = cell.Contains(',') ||
                           cell.Contains('"') ||
                           cell.Contains('\r') ||
                           cell.Contains('\n') ||
                           cell.StartsWith('\'');

        if (needsQuotes)
        {
            return "\"" + cell.Replace("\"", "\"\"") + "\"";
        }

        return cell;
    }

    /// <summary>
    /// Builds full CSV byte content for the given activities and resolved project titles.
    /// </summary>
    public static byte[] BuildActivitiesCsv(
        IReadOnlyList<Activity> activities,
        IReadOnlyDictionary<Guid, string> projectTitles)
    {
        var sb = new StringBuilder(1024 + activities.Count * 256);
        sb.AppendLine(CsvHeader);

        foreach (var a in activities)
        {
            var projectTitle = projectTitles.TryGetValue(a.ProjectId, out var title) ? title : string.Empty;
            var startUtc = a.StartedAt.ToUniversalTime();
            var endUtc = a.EndedAt.ToUniversalTime();
            var duration = endUtc - startUtc;

            var durationMinutes = Math.Round(duration.TotalMinutes, 2).ToString("0.##", CultureInfo.InvariantCulture);
            var durationSeconds = Math.Max(0, (int)Math.Round(duration.TotalSeconds)).ToString(CultureInfo.InvariantCulture);

            sb.Append(a.Id.ToString("D")).Append(',');
            sb.Append(a.UserId.ToString("D")).Append(',');
            sb.Append(a.ProjectId.ToString("D")).Append(',');
            sb.Append(EscapeCsvCell(projectTitle)).Append(',');
            sb.Append(startUtc.ToString("yyyy-MM-ddTHH:mm:ssZ", CultureInfo.InvariantCulture)).Append(',');
            sb.Append(endUtc.ToString("yyyy-MM-ddTHH:mm:ssZ", CultureInfo.InvariantCulture)).Append(',');
            sb.Append(durationMinutes).Append(',');
            sb.Append(durationSeconds).Append(',');
            sb.Append(EscapeCsvCell(a.Description)).Append(',');
            sb.Append(a.Productivity.HasValue ? a.Productivity.Value.ToString(CultureInfo.InvariantCulture) : string.Empty).Append(',');
            sb.Append(a.IdleSeconds.HasValue ? a.IdleSeconds.Value.ToString(CultureInfo.InvariantCulture) : string.Empty).Append(',');
            sb.Append(a.MouseClicks.HasValue ? a.MouseClicks.Value.ToString(CultureInfo.InvariantCulture) : string.Empty).Append(',');
            sb.Append(a.KeyboardHits.HasValue ? a.KeyboardHits.Value.ToString(CultureInfo.InvariantCulture) : string.Empty);
            sb.AppendLine();
        }

        return Encoding.UTF8.GetBytes(sb.ToString());
    }
}
