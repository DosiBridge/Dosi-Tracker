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

/// <summary>The period summary as the model writes it.</summary>
public sealed record PeriodSummaryRaw(
    string Headline,
    IReadOnlyList<AiRawInsight?> Insights,
    IReadOnlyList<AiRawNextStep?> NextSteps,
    IReadOnlyList<string?> Caveats);

/// <summary>
/// "This week at a glance" (team), "Project pulse" (team with a project) and "My week" (self), spec 11:
/// <list type="bullet">
/// <item>team: owners and managers who see the team; up to <see cref="AiAssistConsts.MaxBundleMembers"/> members with
/// their tracked time only (no activity % per member), the rest folded, and how many members tracked nothing;</item>
/// <item>project: the same for one project the caller can see, with its open and completed task counts;</item>
/// <item>self: the caller's own time, no members.</item>
/// </list>
/// Each carries the totals, the previous period of the same length, the days, the top projects and apps, and caveats.
/// At most <see cref="AiAssistConsts.MaxPeriodDays"/> days. A new instance serves each request.
/// </summary>
public class PeriodSummaryFeature : AiInsightFeatureBase<AiPeriodSummaryInput, PeriodSummaryRaw, AiPeriodSummaryDto>, ITransientDependency
{
    public const int MaxInsights = 5;
    public const int MaxNextSteps = 3;
    public const int MaxCaveats = 3;
    public const int MaxProjects = 8;
    public const int MaxApps = 10;

    public const string NoDataHeadline = "Nothing was tracked in this period.";

    public const string ProjectList = "project";
    public const string MembersWithoutTime = "membersWithoutTrackedTime";
    public const string OpenTasks = "openTasks";
    public const string CompletedTasks = "completedTasks";

    private readonly IAiReadLayer _reads;
    private readonly AiScopeGuard _scopeGuard;
    private readonly IClock _clock;

    private AiProject? _project;
    private string _audience = AiUsageRecord.SelfAudience;

    public PeriodSummaryFeature(IAiReadLayer reads, AiScopeGuard scopeGuard, IClock clock)
    {
        _reads = reads;
        _scopeGuard = scopeGuard;
        _clock = clock;
    }

    public override AiPromptSpec Prompt => PeriodSummaryPrompt.Spec;

    public override string? TimeZoneOf(AiPeriodSummaryInput input) => input.TimeZone;

    public override bool IsRegenerate(AiPeriodSummaryInput input) => input.Regenerate;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiPeriodSummaryInput input)
    {
        var team = AiFeatureScopeInput.IsTeam(input.Scope);
        if (team)
        {
            _scopeGuard.EnsureTeamScope(caller);
        }

        var from = ReportCalendar.AsUtc(input.From);
        var to = ReportCalendar.AsUtc(input.To);
        _scopeGuard.EnsureRange(from, to, AiAssistConsts.MaxPeriodDays);

        // A project the caller cannot see is a 404, as on its page.
        _project = input.ProjectId.HasValue ? await _reads.ProjectAsync(caller, input.ProjectId.Value) : null;

        _audience = !team ? AiUsageRecord.SelfAudience
            : _project != null ? AiUsageRecord.ProjectAudience
            : AiUsageRecord.TeamAudience;
        return new AiAudienceScope(_audience, team ? null : caller.UserId, _project?.Id, from, to);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiPeriodSummaryInput input, AiAudienceScope scope)
    {
        var self = scope.Audience == AiUsageRecord.SelfAudience;
        var from = scope.RangeFrom!.Value;
        var to = scope.RangeTo!.Value;
        var query = new AiQuery(from, to, scope.ProjectId, self ? caller.UserId : null);

        var summary = await _reads.SummaryAsync(caller, query);
        if (summary.Blocks == 0 && summary.TrackedMinutes <= 0)
        {
            return new AiBundleWriter(caller.UserId).Build();
        }

        var span = to - from;
        var previous = span <= TimeSpan.FromDays(AiAssistConsts.ComparePreviousMaxDays)
            ? await _reads.SummaryAsync(caller, query with { From = from - span, To = from })
            : null;
        var days = await _reads.DailyAsync(caller, query);
        var apps = await _reads.AppUsageAsync(caller, query);
        var quality = await _reads.DataQualityAsync(caller, query);

        var zone = ReportCalendar.ResolveZone(caller.TimeZone);
        var now = _clock.Now;
        var writer = new AiBundleWriter(self ? new AiPseudonymizer(caller.UserId) : AiPeriodFacts.Pseudonymizer(caller.UserId, summary.Members));
        var subject = self ? writer.AddMember(caller.UserId, AiFeatureScopeInput.NameOf(summary, caller.UserId)) : null;
        string? projectId = null;
        if (_project != null)
        {
            projectId = writer.AddProject(_project.Id, _project.Title);
            writer.SetList(ProjectList, projectId);
        }

        var totals = new JsonObject();
        AiPeriodFacts.AddTotals(writer, totals, summary, subject ?? projectId);
        AiPeriodFacts.AddRange(writer, totals, from, to, zone, now);
        if (previous != null)
        {
            AiPeriodFacts.AddComparison(writer, totals, summary, previous, subject ?? projectId);
        }

        AiPeriodFacts.AddUnmeasured(writer, totals, quality);

        if (!self && _project == null)
        {
            var attendance = await _reads.AttendanceAsync(caller, query);
            totals[MembersWithoutTime] = writer.AddFact("members without tracked time in the period",
                AiPeriodFacts.Count(attendance.Rows.Count(r => r.DaysPresent == 0 && r.WorkedMinutes <= 0)));
        }

        if (_project != null)
        {
            totals[OpenTasks] = writer.AddFact("open tasks of the project",
                AiPeriodFacts.Count((await _reads.TasksAsync(caller, _project.Id, isCompleted: false, max: 1)).TotalCount), projectId);
            totals[CompletedTasks] = writer.AddFact("completed tasks of the project",
                AiPeriodFacts.Count((await _reads.TasksAsync(caller, _project.Id, isCompleted: true, max: 1)).TotalCount), projectId);
        }

        writer.SetList(AiPeriodFacts.TotalsList, totals);

        var (first, last) = AiPeriodFacts.LocalDays(from, to, zone);
        AiPeriodFacts.AddDays(writer, days, DisplayFormat.NeedsYear(first, last, AiPeriodFacts.Today(zone, now)));
        if (_project == null)
        {
            AiPeriodFacts.AddProjects(writer, summary.Projects, summary.TrackedMinutes, MaxProjects);
        }

        AiPeriodFacts.AddApps(writer, apps, MaxApps);
        if (!self)
        {
            AiPeriodFacts.AddMembers(writer, summary.Members);
        }

        foreach (var route in RoutesFor(scope.Audience))
        {
            writer.AddRoute(route);
        }

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        AiFeatureOutput.Categories(bundle, AiDataCategories.Times, AiDataCategories.ActivityPercent);

    public override IReadOnlyList<AiViolation> Validate(PeriodSummaryRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        AiFeatureOutput.ValidateText(violations, "headline", raw.Headline, bundle, required: true);
        AiFeatureOutput.ValidateInsights(violations, "insights", raw.Insights, MaxInsights, AiInsightKinds.Period, bundle);
        AiFeatureOutput.ValidateNextSteps(violations, "nextSteps", raw.NextSteps, MaxNextSteps, bundle);
        AiFeatureOutput.ValidateTexts(violations, "caveats", raw.Caveats, MaxCaveats, bundle);
        return violations;
    }

    /// <summary>"{who} tracked {total} in {period}.", then the top project, the top app and the change from the previous
    /// period when they are known (spec 12.5).</summary>
    public override PeriodSummaryRaw Fallback(AiContextBundle bundle)
    {
        string? Total(string key) => AiFeatureOutput.ListValue(bundle, AiPeriodFacts.TotalsList, key);

        var project = bundle.Lists[ProjectList] is JsonValue p ? (string?)p : null;
        var who = _audience == AiUsageRecord.SelfAudience ? AiFeatureOutput.Ph(AiPseudonymizer.SelfHandle) : "The team";
        var on = project == null ? string.Empty : " on " + AiFeatureOutput.Ph(project);
        var headline = $"{who} tracked {AiFeatureOutput.Ph(Total(AiPeriodFacts.Tracked)!)}{on} in {AiFeatureOutput.Ph(Total(AiPeriodFacts.Period)!)}.";

        var insights = new List<AiRawInsight?>();
        if (AiFeatureOutput.ListItems(bundle, AiPeriodFacts.ProjectsList).FirstOrDefault() is { } top)
        {
            var (id, tracked, share) = (AiFeatureOutput.Value(top, "project")!, AiFeatureOutput.Value(top, "tracked")!, AiFeatureOutput.Value(top, "share")!);
            insights.Add(new AiRawInsight("distribution",
                $"Most tracked time went to {AiFeatureOutput.Ph(id)}: {AiFeatureOutput.Ph(tracked)}, {AiFeatureOutput.Ph(share)} of the total.",
                new[] { tracked, share }, string.Empty));
        }

        if (AiFeatureOutput.ListItems(bundle, AiPeriodFacts.AppsList).FirstOrDefault() is { } app)
        {
            var (id, tracked) = (AiFeatureOutput.Value(app, "app")!, AiFeatureOutput.Value(app, "tracked")!);
            insights.Add(new AiRawInsight("distribution",
                $"The app most in focus was {AiFeatureOutput.Ph(id)}: {AiFeatureOutput.Ph(tracked)}.", new[] { tracked }, string.Empty));
        }

        if (Total(AiPeriodFacts.Change) is { } change && Total(AiPeriodFacts.PreviousTracked) is { } previousTracked)
        {
            insights.Add(new AiRawInsight("trend",
                $"Change from the previous period of the same length ({AiFeatureOutput.Ph(previousTracked)}): {AiFeatureOutput.Ph(change)}.",
                new[] { previousTracked, change }, string.Empty));
        }

        return new PeriodSummaryRaw(headline, insights, Array.Empty<AiRawNextStep?>(), bundle.Caveats.Take(MaxCaveats).ToList());
    }

    public override AiPeriodSummaryDto Render(PeriodSummaryRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = AiFeatureOutput.RenderText(raw.Headline, bundle, AiAssistConsts.MaxHeadlineLength),
        Insights = AiFeatureOutput.RenderInsights(raw.Insights, MaxInsights, bundle),
        NextSteps = AiFeatureOutput.RenderNextSteps(raw.NextSteps, MaxNextSteps, bundle),
        Caveats = AiFeatureOutput.RenderTexts(raw.Caveats, MaxCaveats, AiAssistConsts.MaxCaveatLength, bundle)
    };

    public override AiPeriodSummaryDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = NoDataHeadline
    };

    /// <summary>12 hours while the range includes now (its figures still change), 24 hours otherwise.</summary>
    public override TimeSpan CacheTtl(AiPeriodSummaryInput input, AiContextBundle bundle) =>
        AiPeriodFacts.IncludesNow(input.From, input.To, _clock.Now) ? TimeSpan.FromHours(12) : TimeSpan.FromHours(24);

    /// <summary>The pages a summary may link to.</summary>
    public static IReadOnlyList<string> RoutesFor(string audience) => audience switch
    {
        AiUsageRecord.SelfAudience => new[] { AiRouteIds.Timesheet, AiRouteIds.Activities, AiRouteIds.Dashboard },
        AiUsageRecord.ProjectAudience => new[]
        {
            AiRouteIds.Projects, AiRouteIds.Timesheet, AiRouteIds.ForReport("projects"), AiRouteIds.ForReport("time-activity")
        },
        _ => new[]
        {
            AiRouteIds.Timesheet, AiRouteIds.Team, AiRouteIds.Monitor, AiRouteIds.Projects, AiRouteIds.Reports,
            AiRouteIds.ForReport("time-activity"), AiRouteIds.ForReport("weekly"), AiRouteIds.ForReport("projects"),
            AiRouteIds.ForReport("apps-urls"), AiRouteIds.ForReport("attendance")
        }
    };
}
