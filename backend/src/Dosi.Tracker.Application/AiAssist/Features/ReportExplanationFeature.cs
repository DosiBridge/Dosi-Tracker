using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Timing;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>The report explanation as the model writes it.</summary>
public sealed record ReportExplanationRaw(
    string Summary,
    IReadOnlyList<AiRawInsight?> KeyFindings,
    IReadOnlyList<string?> Caveats,
    IReadOnlyList<string?> QuestionsToExplore);

/// <summary>
/// "Explain this report" (spec 11): owners and managers who see the team, for the nine reports of the web app's catalog
/// (never payroll), with the report's own filters. What each report reads is <see cref="AiReportFacts.SourcesFor"/>;
/// every explanation also gets the totals and, for ranges of 92 days or less, the previous period. At most 12 members,
/// 10 projects, 15 apps and 31 points (weeks above 31 days, months above 92). Device names and OS versions are never
/// read. A new instance serves each request.
/// </summary>
public class ReportExplanationFeature
    : AiInsightFeatureBase<AiReportExplanationInput, ReportExplanationRaw, AiReportExplanationDto>, ITransientDependency
{
    public const int MaxKeyFindings = 6;
    public const int MaxCaveats = 4;
    public const int MaxQuestions = 3;
    public const int MaxFallbackRows = 3;

    public const string ReportList = "report";
    public const string NoDataSummary = "Nothing was tracked in this report's range.";

    private readonly IAiReadLayer _reads;
    private readonly AiScopeGuard _scopeGuard;
    private readonly IClock _clock;

    private AiProject? _project;
    private string _slug = string.Empty;
    private string? _memberHandle;
    private string? _projectRef;

    public ReportExplanationFeature(IAiReadLayer reads, AiScopeGuard scopeGuard, IClock clock)
    {
        _reads = reads;
        _scopeGuard = scopeGuard;
        _clock = clock;
    }

    public override AiPromptSpec Prompt => ReportExplanationPrompt.Spec;

    public override string? TimeZoneOf(AiReportExplanationInput input) => input.TimeZone;

    public override bool IsRegenerate(AiReportExplanationInput input) => input.Regenerate;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiReportExplanationInput input)
    {
        _scopeGuard.EnsureTeamScope(caller);
        _slug = _scopeGuard.EnsureReportSlug(input.ReportSlug);

        var from = ReportCalendar.AsUtc(input.From);
        var to = ReportCalendar.AsUtc(input.To);
        _scopeGuard.EnsureRange(from, to, AiAssistConsts.MaxReportDays);

        Guid? subject = input.UserId.HasValue ? await _scopeGuard.EnsureMemberTargetAsync(caller, input.UserId) : null;
        _project = input.ProjectId.HasValue ? await _reads.ProjectAsync(caller, input.ProjectId.Value) : null;

        var audience = subject == caller.UserId ? AiUsageRecord.SelfAudience
            : subject.HasValue ? AiUsageRecord.MemberAudience
            : _project != null ? AiUsageRecord.ProjectAudience
            : AiUsageRecord.TeamAudience;
        var dayBasis = ReportCalendar.IsMemberBasis(input.DayBasis) ? ReportDayBasis.Member : ReportDayBasis.Zone;
        return new AiAudienceScope(audience, subject, _project?.Id, from, to, _slug, dayBasis);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiReportExplanationInput input, AiAudienceScope scope)
    {
        var from = scope.RangeFrom!.Value;
        var to = scope.RangeTo!.Value;
        var sources = AiReportFacts.SourcesFor(_slug);
        var query = new AiQuery(from, to, scope.ProjectId, scope.SubjectUserId, DayBasis: scope.DayBasis);

        var summary = await _reads.SummaryAsync(caller, query);
        if (summary.Blocks == 0 && summary.TrackedMinutes <= 0)
        {
            return new AiBundleWriter(caller.UserId).Build();
        }

        var span = to - from;
        var previous = span <= TimeSpan.FromDays(AiAssistConsts.ComparePreviousMaxDays)
            ? await _reads.SummaryAsync(caller, query with { From = from - span, To = from })
            : null;
        var days = sources.HasFlag(AiReportSources.Daily) ? await _reads.DailyAsync(caller, query) : null;
        var memberDays = sources.HasFlag(AiReportSources.MemberDays) ? await _reads.UserDailyAsync(caller, query) : null;
        var hourly = sources.HasFlag(AiReportSources.Hourly) ? await _reads.HourlyAsync(caller, query) : null;
        var apps = sources.HasFlag(AiReportSources.AppUsage) ? await _reads.AppUsageAsync(caller, query) : null;
        var attendance = sources.HasFlag(AiReportSources.Attendance) ? await _reads.AttendanceAsync(caller, query) : null;
        var devices = sources.HasFlag(AiReportSources.Devices) ? await _reads.DevicesAsync(caller, query) : null;
        var names = attendance != null || devices != null ? await NamesAsync(caller, summary) : new Dictionary<Guid, string>();

        // Handles by tracked minutes; members without time (attendance) or only on a device row come after them.
        var weights = summary.Members.Select(m => new AiMemberWeight(m.UserId, m.TrackedMinutes))
            .Concat(attendance?.Rows.Select(r => new AiMemberWeight(r.UserId, 0)) ?? Array.Empty<AiMemberWeight>())
            .Concat(devices?.Select(d => new AiMemberWeight(d.UserId, 0)) ?? Array.Empty<AiMemberWeight>());
        var writer = new AiBundleWriter(AiPseudonymizer.ForCard(caller.UserId, weights));
        writer.SetList(ReportList, _slug);

        _memberHandle = scope.SubjectUserId is { } subjectId
            ? writer.AddMember(subjectId, AiFeatureScopeInput.NameOf(summary, subjectId))
            : null;
        _projectRef = _project == null ? null : writer.AddProject(_project.Id, _project.Title);
        var subject = _memberHandle ?? _projectRef;

        var zone = ReportCalendar.ResolveZone(caller.TimeZone);
        var now = _clock.Now;
        var totals = new JsonObject();
        AiPeriodFacts.AddTotals(writer, totals, summary, subject);
        AiPeriodFacts.AddRange(writer, totals, from, to, zone, now);
        if (previous != null)
        {
            AiPeriodFacts.AddComparison(writer, totals, summary, previous, subject);
        }

        writer.SetList(AiPeriodFacts.TotalsList, totals);

        if (days != null)
        {
            var (first, last) = AiPeriodFacts.LocalDays(from, to, zone);
            AiPeriodFacts.AddPoints(writer, days, AiPeriodFacts.RangeDays(from, to),
                DisplayFormat.NeedsYear(first, last, AiPeriodFacts.Today(zone, now)));
        }

        if (_slug is not ("apps-urls" or "attendance" or "devices"))
        {
            AiPeriodFacts.AddMembers(writer, summary.Members, withActivity: _slug == "productivity");
        }

        if (_slug is "projects" or "work-report" or "analytics")
        {
            AiPeriodFacts.AddProjects(writer, summary.Projects, summary.TrackedMinutes, AiAssistConsts.MaxBundleProjects);
        }

        if (_slug == "projects")
        {
            var listed = summary.Projects.Where(p => p.TrackedMinutes > 0)
                .OrderByDescending(p => p.TrackedMinutes).ThenBy(p => p.ProjectId)
                .Take(AiAssistConsts.MaxBundleProjects).Select(p => p.ProjectId).ToHashSet();
            AiReportFacts.AddMemberProjects(writer, summary.MemberProjects, listed);
        }

        if (memberDays != null)
        {
            AiReportFacts.AddMemberDays(writer, memberDays);
        }

        if (hourly != null)
        {
            AiReportFacts.AddHours(writer, hourly);
        }

        if (apps != null)
        {
            AiPeriodFacts.AddApps(writer, apps, AiAssistConsts.MaxBundleApps);
        }

        if (attendance != null)
        {
            AiReportFacts.AddAttendance(writer, attendance, names);
        }

        if (devices != null)
        {
            AiReportFacts.AddDevices(writer, devices, names);
        }

        writer.AddRoute(AiRouteIds.ForReport(_slug)).AddRoute(AiRouteIds.Reports).AddRoute(AiRouteIds.Timesheet).AddRoute(AiRouteIds.Team);
        if (_slug is "projects" or "work-report")
        {
            writer.AddRoute(AiRouteIds.Projects);
        }

        if (_slug == "devices")
        {
            writer.AddRoute(AiRouteIds.Download);
        }

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        _slug == "devices"
            ? AiFeatureOutput.Categories(bundle, AiDataCategories.Times, AiDataCategories.ActivityPercent, AiDataCategories.AgentInfo)
            : AiFeatureOutput.Categories(bundle, AiDataCategories.Times, AiDataCategories.ActivityPercent);

    public override IReadOnlyList<AiViolation> Validate(ReportExplanationRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        AiFeatureOutput.ValidateText(violations, "summary", raw.Summary, bundle, required: true);
        AiFeatureOutput.ValidateInsights(violations, "keyFindings", raw.KeyFindings, MaxKeyFindings, AiInsightKinds.Report, bundle);
        AiFeatureOutput.ValidateTexts(violations, "caveats", raw.Caveats, MaxCaveats, bundle);
        AiFeatureOutput.ValidateTexts(violations, "questionsToExplore", raw.QuestionsToExplore, MaxQuestions, bundle);
        return violations;
    }

    /// <summary>A totals sentence and the top 3 rows of the report's main list (spec 12.5).</summary>
    public override ReportExplanationRaw Fallback(AiContextBundle bundle)
    {
        string? Total(string key) => AiFeatureOutput.ListValue(bundle, AiPeriodFacts.TotalsList, key);

        var tracked = AiFeatureOutput.Ph(Total(AiPeriodFacts.Tracked)!);
        var period = AiFeatureOutput.Ph(Total(AiPeriodFacts.Period)!);
        var summary = _memberHandle != null ? $"{AiFeatureOutput.Ph(_memberHandle)} tracked {tracked} in {period}."
            : _projectRef != null ? $"The team tracked {tracked} on {AiFeatureOutput.Ph(_projectRef)} in {period}."
            : $"The team tracked {tracked} in {period}.";

        var findings = AiFeatureOutput.ListItems(bundle, AiReportFacts.MainListFor(_slug))
            .Take(MaxFallbackRows)
            .Select(FallbackRow)
            .Where(f => f != null)
            .ToList();

        return new ReportExplanationRaw(summary, findings, bundle.Caveats.Take(MaxCaveats).ToList(), Array.Empty<string?>());
    }

    public override AiReportExplanationDto Render(ReportExplanationRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Summary = AiFeatureOutput.RenderText(raw.Summary, bundle, AiAssistConsts.MaxSummaryLength),
        KeyFindings = AiFeatureOutput.RenderInsights(raw.KeyFindings, MaxKeyFindings, bundle),
        Caveats = AiFeatureOutput.RenderTexts(raw.Caveats, MaxCaveats, AiAssistConsts.MaxCaveatLength, bundle),
        QuestionsToExplore = AiFeatureOutput.RenderTexts(raw.QuestionsToExplore, MaxQuestions, AiAssistConsts.MaxItemLength, bundle)
    };

    public override AiReportExplanationDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Summary = NoDataSummary
    };

    public override TimeSpan CacheTtl(AiReportExplanationInput input, AiContextBundle bundle) => TimeSpan.FromHours(24);

    /// <summary>Display names of the workspace's members: the summary's, then the roster's for members without time.</summary>
    protected virtual async Task<IReadOnlyDictionary<Guid, string>> NamesAsync(AiCallerContext caller, AiSummary summary)
    {
        var names = summary.Members.ToDictionary(m => m.UserId, m => m.DisplayName);
        foreach (var member in await _reads.RosterAsync(caller))
        {
            names.TryAdd(member.UserId, member.DisplayName);
        }

        return names;
    }

    private static AiRawInsight? FallbackRow(JsonObject row)
    {
        string? V(string key) => AiFeatureOutput.Value(row, key);
        string P(string id) => AiFeatureOutput.Ph(id);

        if (V("project") is { } project && V("tracked") is { } projectTime && V("share") is { } projectShare)
        {
            return new AiRawInsight("distribution", $"{P(project)}: {P(projectTime)}, {P(projectShare)} of the tracked time.",
                new[] { projectTime, projectShare }, string.Empty);
        }

        if (V("app") is { } app && V("tracked") is { } appTime && V("share") is { } appShare)
        {
            return new AiRawInsight("apps", $"{P(app)} was in focus for {P(appTime)}, {P(appShare)} of the time with an app in focus.",
                new[] { appTime, appShare }, string.Empty);
        }

        if (V("member") is not { } member)
        {
            return null;
        }

        if (V("present") is { } present && V("absent") is { } absent)
        {
            return new AiRawInsight("attendance",
                $"{P(member)}: {P(present)} working days with tracked time and {P(absent)} without.", new[] { present, absent }, string.Empty);
        }

        if (V("platform") is { } platform && V("agentVersion") is { } version)
        {
            return new AiRawInsight("devices", $"{P(member)} tracked with the {P(platform)} agent, version {P(version)}.",
                new[] { platform, version }, string.Empty);
        }

        return V("tracked") is { } tracked
            ? new AiRawInsight("distribution", $"{P(member)} tracked {P(tracked)}.", new[] { tracked }, string.Empty)
            : null;
    }
}
