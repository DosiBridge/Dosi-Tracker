using System;
using System.Collections.Generic;
using System.Linq;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>
/// The pages an AI answer may link to. The server only ever emits these ids; the web app maps each to its page and
/// still checks that the reader may open it. Links are never built from model text.
/// </summary>
public static class AiRouteIds
{
    public const string Dashboard = "dashboard";
    public const string Monitor = "monitor";
    public const string Timesheet = "timesheet";
    public const string Team = "team";
    public const string Projects = "projects";

    /// <summary>One project's page (its id travels as the route argument).</summary>
    public const string ProjectDetail = "projects.detail";

    public const string Activities = "activities";
    public const string Download = "download";
    public const string InstallWindows = "install.windows";
    public const string InstallMacos = "install.macos";
    public const string InstallLinux = "install.linux";
    public const string Settings = "settings";
    public const string SettingsWorkspace = "settings.workspace";
    public const string SettingsPrivacy = "settings.privacy";
    public const string Reports = "reports";
    public const string Billing = "billing";

    /// <summary>The prefix of one report's page: <c>reports.{slug}</c>.</summary>
    public const string ReportPrefix = "reports.";

    /// <summary>The web app's report catalog.</summary>
    public static readonly IReadOnlyList<string> ReportSlugs = new[]
    {
        "work-report", "analytics", "time-activity", "weekly", "projects", "productivity", "apps-urls", "attendance",
        "devices", "payroll"
    };

    /// <summary>Every allowed id, report pages included.</summary>
    public static readonly IReadOnlySet<string> All = new HashSet<string>(
        new[]
        {
            Dashboard, Monitor, Timesheet, Team, Projects, ProjectDetail, Activities, Download, InstallWindows,
            InstallMacos, InstallLinux, Settings, SettingsWorkspace, SettingsPrivacy, Reports, Billing
        }.Concat(ReportSlugs.Select(ForReport)),
        StringComparer.Ordinal);

    /// <summary>True for an allowlisted id (exact, case-sensitive).</summary>
    public static bool IsAllowed(string? routeId) => routeId != null && All.Contains(routeId);

    /// <summary>The id of a report's page.</summary>
    public static string ForReport(string slug) => ReportPrefix + slug;
}
