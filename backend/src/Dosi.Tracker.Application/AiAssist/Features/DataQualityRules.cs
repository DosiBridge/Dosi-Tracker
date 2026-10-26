using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>What the data-quality rules look at (all read as the caller, so a member's own review holds only their own
/// rows).</summary>
/// <param name="Zone">The zone days and weekdays are counted in.</param>
/// <param name="FromUtc">The review's range start (inclusive).</param>
/// <param name="ToUtc">The review's range end (exclusive).</param>
/// <param name="Team">A team review (never-tracked and outdated-agent are for teams only).</param>
/// <param name="Kinds">The kinds to look for.</param>
/// <param name="Roster">The members the caller may see, with when they joined and whether they are clients.</param>
/// <param name="MemberDays">Member-days of the range with tracked time (local dates of <paramref name="Zone"/>).</param>
/// <param name="Quality">The data-quality rows of the range.</param>
/// <param name="RecentQuality">The data-quality rows of the <see cref="DataQualityRules.NeverTrackedWindowDays"/> days
/// before <paramref name="ToUtc"/> (never-tracked).</param>
/// <param name="Devices">The devices of the range (outdated-agent).</param>
public sealed record DataQualityContext(
    TimeZoneInfo Zone,
    DateTime FromUtc,
    DateTime ToUtc,
    DateTime NowUtc,
    bool Team,
    Guid CallerId,
    IReadOnlyCollection<string> Kinds,
    IReadOnlyList<AiRosterEntry> Roster,
    IReadOnlyList<AiMemberDayRow> MemberDays,
    IReadOnlyList<AiDataQualityRow> Quality,
    IReadOnlyList<AiDataQualityRow> RecentQuality,
    IReadOnlyList<AiDeviceRow> Devices);

/// <summary>A figure behind a candidate: its key in the candidate (<c>first</c>, <c>threshold</c> …), a label without
/// digits and the formatted value.</summary>
public sealed record DataQualityFact(string Key, string Label, string Display);

/// <summary>Something the reader can do about a candidate, and the page it opens (null: nothing to open).</summary>
public sealed record DataQualityAction(string Id, string? RouteId);

/// <summary>One rule hit about one member.</summary>
/// <param name="Weight">The member's tracked minutes in the range (orders candidates and handles).</param>
public sealed record DataQualityCandidate(
    string Kind,
    string Severity,
    Guid UserId,
    string DisplayName,
    double Weight,
    IReadOnlyList<DataQualityFact> Facts,
    IReadOnlyList<DataQualityAction> Actions);

/// <summary>
/// The data-quality rules of spec 11: pure and deterministic, so the model only explains what a fixed rule found and
/// never decides what is wrong. No rule ranks people or looks at how much anyone worked.
/// <list type="bullet">
/// <item>data-gap: a member with time earlier in the range has none on <see cref="GapWeekdays"/> or more consecutive
/// weekdays (Monday to Friday in the zone, after the day they joined, before today);</item>
/// <item>clock-skew: a device clock more than <see cref="ClockSkewSeconds"/> seconds off;</item>
/// <item>unmeasured-activity: more than <see cref="UnmeasuredShare"/> of the tracked minutes without activity %;</item>
/// <item>software-input: <see cref="SoftwareInputBlocks"/> or more blocks with software-generated input (benign:
/// remote desktop, accessibility tools, automation);</item>
/// <item>never-tracked (team): an active member who is not a client or an owner, joined at least
/// <see cref="NeverTrackedJoinedDays"/> days before the range end, with no blocks in the
/// <see cref="NeverTrackedWindowDays"/> days before it;</item>
/// <item>outdated-agent (team): a member's latest device on a platform runs an agent older than the newest seen on that
/// platform in the workspace (numeric version prefixes; unparsable versions are skipped).</item>
/// </list>
/// Candidates are ordered by kind, then by the member's tracked time, then by id; at most <see cref="MaxCandidates"/>.
/// </summary>
public static class DataQualityRules
{
    public const int GapWeekdays = 3;
    public const int ClockSkewSeconds = 300;
    public const double UnmeasuredShare = 0.30;
    public const int SoftwareInputBlocks = 3;
    public const int NeverTrackedJoinedDays = 3;
    public const int NeverTrackedWindowDays = 30;
    public const int MaxCandidates = 20;

    public const string CheckAgentRunning = "check-agent-running";
    public const string OpenDownload = "open-download";
    public const string SyncDeviceClock = "sync-device-clock";
    public const string ReviewCaptureSettings = "review-capture-settings";
    public const string CheckOsPermissions = "check-os-permissions";
    public const string SendInstallLink = "send-install-link";
    public const string OpenTeam = "open-team";
    public const string UpdateAgent = "update-agent";

    /// <summary>The actions each kind allows, with the page each opens.</summary>
    public static readonly IReadOnlyDictionary<string, IReadOnlyList<DataQualityAction>> Actions =
        new Dictionary<string, IReadOnlyList<DataQualityAction>>(StringComparer.Ordinal)
        {
            [AiDataQualityKinds.DataGap] = new[] { new DataQualityAction(CheckAgentRunning, AiRouteIds.Download), new DataQualityAction(OpenDownload, AiRouteIds.Download) },
            [AiDataQualityKinds.ClockSkew] = new[] { new DataQualityAction(SyncDeviceClock, null) },
            [AiDataQualityKinds.UnmeasuredActivity] = new[]
            {
                new DataQualityAction(ReviewCaptureSettings, AiRouteIds.Projects), new DataQualityAction(CheckOsPermissions, AiRouteIds.Download)
            },
            [AiDataQualityKinds.SoftwareInput] = Array.Empty<DataQualityAction>(),
            [AiDataQualityKinds.NeverTracked] = new[] { new DataQualityAction(SendInstallLink, AiRouteIds.Download), new DataQualityAction(OpenTeam, AiRouteIds.Team) },
            [AiDataQualityKinds.OutdatedAgent] = new[] { new DataQualityAction(UpdateAgent, AiRouteIds.Download) }
        };

    /// <summary>notice before info.</summary>
    public static int SeverityRank(string severity) => severity == AiDataQualitySeverities.Notice ? 0 : 1;

    public static IReadOnlyList<DataQualityCandidate> Find(DataQualityContext context)
    {
        var weights = context.Quality.GroupBy(q => q.UserId).ToDictionary(g => g.Key, g => g.Sum(q => q.TrackedMinutes));
        var names = context.Roster.ToDictionary(r => r.UserId, r => r.DisplayName);
        string Name(Guid userId, string fallback) => names.TryGetValue(userId, out var name) ? name : fallback;
        double Weight(Guid userId) => weights.GetValueOrDefault(userId);

        var candidates = new List<DataQualityCandidate>();
        bool Wants(string kind) => context.Kinds.Contains(kind) && (context.Team || AiDataQualityKinds.Self.Contains(kind));

        if (Wants(AiDataQualityKinds.DataGap))
        {
            candidates.AddRange(DataGaps(context, Name, Weight));
        }

        foreach (var row in context.Quality.OrderBy(q => q.UserId))
        {
            if (Wants(AiDataQualityKinds.ClockSkew) && row.MaxAbsClockSkewSeconds is > ClockSkewSeconds)
            {
                candidates.Add(Candidate(AiDataQualityKinds.ClockSkew, AiDataQualitySeverities.Notice, row.UserId, Name(row.UserId, row.UserName), Weight(row.UserId),
                    new DataQualityFact("skew", "largest device clock difference", Seconds(row.MaxAbsClockSkewSeconds.Value)),
                    new DataQualityFact("threshold", "clock difference threshold", Seconds(ClockSkewSeconds))));
            }

            if (Wants(AiDataQualityKinds.UnmeasuredActivity) && row.TrackedMinutes > 0 && row.UnmeasuredMinutes / row.TrackedMinutes > UnmeasuredShare)
            {
                candidates.Add(Candidate(AiDataQualityKinds.UnmeasuredActivity, AiDataQualitySeverities.Info, row.UserId, Name(row.UserId, row.UserName), Weight(row.UserId),
                    new DataQualityFact("share", "share of tracked time without activity measurement", AiPeriodFacts.Share(row.UnmeasuredMinutes, row.TrackedMinutes)),
                    new DataQualityFact("unmeasured", "tracked time without activity measurement", DisplayFormat.Duration(row.UnmeasuredMinutes)),
                    new DataQualityFact("threshold", "unmeasured share threshold", DisplayFormat.Percent(UnmeasuredShare * 100))));
            }

            if (Wants(AiDataQualityKinds.SoftwareInput) && row.BlocksWithInjectedInput >= SoftwareInputBlocks)
            {
                candidates.Add(Candidate(AiDataQualityKinds.SoftwareInput, AiDataQualitySeverities.Info, row.UserId, Name(row.UserId, row.UserName), Weight(row.UserId),
                    new DataQualityFact("blocks", "blocks with software-generated input", AiPeriodFacts.Count(row.BlocksWithInjectedInput)),
                    new DataQualityFact("threshold", "software input threshold in blocks", AiPeriodFacts.Count(SoftwareInputBlocks))));
            }
        }

        if (Wants(AiDataQualityKinds.NeverTracked))
        {
            candidates.AddRange(NeverTracked(context));
        }

        if (Wants(AiDataQualityKinds.OutdatedAgent))
        {
            candidates.AddRange(OutdatedAgents(context, Name, Weight));
        }

        return candidates
            .OrderBy(c => KindRank(c.Kind))
            .ThenByDescending(c => c.Weight)
            .ThenBy(c => c.UserId)
            .Take(MaxCandidates)
            .ToList();
    }

    private static IEnumerable<DataQualityCandidate> DataGaps(DataQualityContext context, Func<Guid, string, string> name, Func<Guid, double> weight)
    {
        var clock = LocalClock.Of(context.Zone);
        var (firstDay, lastDay) = AiPeriodFacts.LocalDays(context.FromUtc, context.ToUtc, context.Zone);
        var today = clock.ToLocal(context.NowUtc).Date;
        // Today is not over: only past weekdays can be without tracked time.
        var end = lastDay < today ? lastDay : today.AddDays(-1);
        var withYear = DisplayFormat.NeedsYear(firstDay, lastDay, today);
        var roster = context.Roster.ToDictionary(r => r.UserId);

        foreach (var member in context.MemberDays.Where(d => d.TrackedMinutes > 0).GroupBy(d => d.UserId).OrderBy(g => g.Key))
        {
            if (!roster.TryGetValue(member.Key, out var entry))
            {
                // No longer an active member (or not one the caller may see).
                continue;
            }

            var active = member.Select(d => d.Date.Date).ToHashSet();
            var joined = clock.ToLocal(entry.JoinedAt).Date;
            var start = active.Min().AddDays(1);
            if (start <= joined)
            {
                start = joined.AddDays(1);
            }

            var (runStart, runLength, bestStart, bestEnd, bestLength) = (start, 0, start, start, 0);
            for (var day = start; day <= end; day = day.AddDays(1))
            {
                if (day.DayOfWeek is DayOfWeek.Saturday or DayOfWeek.Sunday)
                {
                    continue;
                }

                if (active.Contains(day))
                {
                    runLength = 0;
                    continue;
                }

                if (runLength == 0)
                {
                    runStart = day;
                }

                runLength++;
                if (runLength > bestLength)
                {
                    (bestStart, bestEnd, bestLength) = (runStart, day, runLength);
                }
            }

            if (bestLength < GapWeekdays)
            {
                continue;
            }

            yield return Candidate(AiDataQualityKinds.DataGap, AiDataQualitySeverities.Notice, member.Key, name(member.Key, entry.UserName), weight(member.Key),
                new DataQualityFact("first", "first weekday without tracked time", DisplayFormat.Day(bestStart, withYear)),
                new DataQualityFact("last", "last weekday without tracked time", DisplayFormat.Day(bestEnd, withYear)),
                new DataQualityFact("weekdays", "weekdays without tracked time in a row", AiPeriodFacts.Count(bestLength)),
                new DataQualityFact("threshold", "gap threshold in weekdays", AiPeriodFacts.Count(GapWeekdays)));
        }
    }

    private static IEnumerable<DataQualityCandidate> NeverTracked(DataQualityContext context)
    {
        var tracked = context.RecentQuality.Where(q => q.Blocks > 0).Select(q => q.UserId).ToHashSet();
        var clock = LocalClock.Of(context.Zone);
        var latestJoin = ReportCalendar.AsUtc(context.ToUtc).AddDays(-NeverTrackedJoinedDays);

        foreach (var member in context.Roster.OrderBy(r => r.UserId))
        {
            // Owners often only manage; clients follow projects and never track; the caller is not told about themselves.
            if (member.IsClient || member.Role == WorkspaceRoles.Owner || member.UserId == context.CallerId ||
                ReportCalendar.AsUtc(member.JoinedAt) > latestJoin || tracked.Contains(member.UserId))
            {
                continue;
            }

            var joined = clock.ToLocal(member.JoinedAt).Date;
            yield return Candidate(AiDataQualityKinds.NeverTracked, AiDataQualitySeverities.Notice, member.UserId, member.DisplayName, 0,
                new DataQualityFact("joined", "joined the workspace", DisplayFormat.Day(joined, withYear: true)),
                new DataQualityFact("window", "period checked for tracked time", AiPeriodFacts.Count(NeverTrackedWindowDays) + " days"));
        }
    }

    private static IEnumerable<DataQualityCandidate> OutdatedAgents(DataQualityContext context, Func<Guid, string, string> name, Func<Guid, double> weight)
    {
        var parsed = context.Devices
            .Select(d => (Device: d, Platform: AiReportFacts.Platform(d.Platform), Version: AiReportFacts.ParseVersion(d.AgentVersion)))
            .Where(d => d.Platform != "unknown" && d.Version != null)
            .ToList();

        foreach (var platform in parsed.GroupBy(d => d.Platform).OrderBy(g => g.Key, StringComparer.Ordinal))
        {
            var newest = platform.Max(d => d.Version)!;
            foreach (var latest in platform
                         .GroupBy(d => d.Device.UserId)
                         .Select(g => g.OrderByDescending(d => d.Device.LastActivityAt).ThenByDescending(d => d.Version).First())
                         .Where(d => d.Version! < newest)
                         .OrderBy(d => d.Device.UserId))
            {
                var userId = latest.Device.UserId;
                yield return Candidate(AiDataQualityKinds.OutdatedAgent, AiDataQualitySeverities.Info, userId, name(userId, latest.Device.UserName), weight(userId),
                    new DataQualityFact("platform", "agent platform", platform.Key),
                    new DataQualityFact("version", "agent version in use", AiReportFacts.AgentVersion(latest.Device.AgentVersion)),
                    new DataQualityFact("newest", "newest agent version seen in the workspace", newest.ToString()));
            }
        }
    }

    private static DataQualityCandidate Candidate(
        string kind, string severity, Guid userId, string displayName, double weight, params DataQualityFact[] facts) =>
        new(kind, severity, userId, displayName, weight, facts, Actions[kind]);

    private static int KindRank(string kind)
    {
        for (var i = 0; i < AiDataQualityKinds.All.Count; i++)
        {
            if (AiDataQualityKinds.All[i] == kind)
            {
                return i;
            }
        }

        return int.MaxValue;
    }

    /// <summary>"45 s", or minutes from a minute on ("7 min").</summary>
    private static string Seconds(int seconds)
    {
        var magnitude = Math.Abs((long)seconds);
        return magnitude < 60 ? AiPeriodFacts.Count(magnitude) + " s" : DisplayFormat.Duration(magnitude / 60.0);
    }
}
