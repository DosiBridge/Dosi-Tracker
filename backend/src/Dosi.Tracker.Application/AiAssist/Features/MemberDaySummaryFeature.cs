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

/// <summary>A highlight has no route: <see cref="AiRawInsight"/> without one, as the schema asks for it.</summary>
public sealed record AiRawHighlight(string Kind, string Text, IReadOnlyList<string> FactRefs);

/// <summary>
/// "Explain this day" (spec 11): one member's local day in the caller's zone — the caller's own, or (for a caller who
/// sees the team) another active member's, who is then <c>M1</c>. The bundle holds the day's totals, rhythm, breaks,
/// projects (at most 6), apps (at most 8), morning / afternoon / evening shares, how blocks ended, unmeasured and
/// software-input time and the comparison with the previous week; notes only when the workspace allows notes, window
/// titles only when it allows window titles. A new instance serves each request.
/// </summary>
public class MemberDaySummaryFeature
    : AiInsightFeatureBase<AiMemberDaySummaryInput, MemberDaySummaryRaw, AiMemberDaySummaryDto>, ITransientDependency
{
    public const int MaxHighlights = 5;
    public const int MaxCaveats = 3;

    public const string NoDataHeadline = "Nothing was tracked on this day.";

    private readonly IAiReadLayer _reads;
    private readonly AiScopeGuard _scopeGuard;
    private readonly IClock _clock;

    private DateTime _dayEndUtc;

    public MemberDaySummaryFeature(IAiReadLayer reads, AiScopeGuard scopeGuard, IClock clock)
    {
        _reads = reads;
        _scopeGuard = scopeGuard;
        _clock = clock;
    }

    public override AiPromptSpec Prompt => MemberDaySummaryPrompt.Spec;

    public override string? TimeZoneOf(AiMemberDaySummaryInput input) => input.TimeZone;

    public override bool IsRegenerate(AiMemberDaySummaryInput input) => input.Regenerate;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiMemberDaySummaryInput input)
    {
        var subject = await _scopeGuard.EnsureMemberTargetAsync(caller, input.UserId);
        var day = AiPeriodResolver.ResolveDay(input.Date, caller.TimeZone);
        var audience = subject == caller.UserId ? AiUsageRecord.SelfAudience : AiUsageRecord.MemberAudience;
        _dayEndUtc = day.ToUtc;
        return new AiAudienceScope(audience, subject, RangeFrom: day.FromUtc, RangeTo: day.ToUtc);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiMemberDaySummaryInput input, AiAudienceScope scope)
    {
        var subjectId = scope.SubjectUserId!.Value;
        var day = AiPeriodResolver.ResolveDay(input.Date, caller.TimeZone);
        var query = new AiQuery(day.FromUtc, day.ToUtc, UserId: subjectId);

        var summary = await _reads.SummaryAsync(caller, query);
        var blocks = await _reads.DayBlocksAsync(caller, subjectId, day.FromUtc, day.ToUtc);
        if (summary.Blocks == 0 && blocks.Count == 0)
        {
            return new AiBundleWriter(caller.UserId).Build();
        }

        var hourly = await _reads.HourlyAsync(caller, query);
        var apps = await _reads.AppUsageAsync(caller, query);
        var zone = ReportCalendar.ResolveZone(day.TimeZone);
        var calendar = new ReportCalendar(zone, memberBasis: false);
        var previousWeek = await _reads.UserDailyAsync(caller,
            query with { From = calendar.StartOfDayUtc(day.FirstDay.AddDays(-7)), To = day.FromUtc });

        // The member the day is about is the only person in the bundle: ME for the caller, M1 for anyone else.
        var writer = new AiBundleWriter(caller.UserId);
        var subject = writer.AddMember(subjectId, AiFeatureScopeInput.NameOf(summary, subjectId));

        var totals = new JsonObject();
        AiPeriodFacts.AddTotals(writer, totals, summary, subject);
        AiDayFacts.AddDay(writer, totals, day.FirstDay, zone, day.FromUtc, _clock.Now);
        AiDayFacts.AddRhythm(writer, totals, blocks, zone, subject);
        AiDayFacts.AddMeasurement(writer, totals, blocks);
        AiDayFacts.AddWeekComparison(writer, totals, previousWeek, subjectId, summary.TrackedMinutes, subject);
        writer.SetList(AiPeriodFacts.TotalsList, totals);

        AiPeriodFacts.AddProjects(writer, summary.Projects, summary.TrackedMinutes, AiDayFacts.MaxProjects);
        AiPeriodFacts.AddApps(writer, apps, AiDayFacts.MaxApps);
        AiDayFacts.AddDayParts(writer, hourly);
        AiDayFacts.AddEndReasons(writer, blocks);

        // The read layer leaves notes and titles null unless the workspace allows them; the policy is checked again here.
        if (caller.Policy.AllowNotes)
        {
            AiDayFacts.AddNotes(writer, blocks);
        }

        if (caller.Policy.AllowWindowTitles)
        {
            AiDayFacts.AddTitles(writer, blocks);
        }

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        AiFeatureOutput.Categories(bundle, AiDataCategories.Times, AiDataCategories.ActivityPercent);

    public override IReadOnlyList<AiViolation> Validate(MemberDaySummaryRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        AiFeatureOutput.ValidateText(violations, "headline", raw.Headline, bundle, required: true);
        AiFeatureOutput.ValidateText(violations, "summary", raw.Summary, bundle, required: true);
        AiFeatureOutput.ValidateInsights(violations, "highlights", raw.AsInsights(), MaxHighlights, AiInsightKinds.MemberDay, bundle,
            withRoutes: false);
        AiFeatureOutput.ValidateTexts(violations, "caveats", raw.Caveats, MaxCaveats, bundle);
        return violations;
    }

    /// <summary>"{M1} tracked {F1} on {day}." then the first start and last end, and where most time went (spec 12.5).</summary>
    public override MemberDaySummaryRaw Fallback(AiContextBundle bundle)
    {
        string? Total(string key) => AiFeatureOutput.ListValue(bundle, AiPeriodFacts.TotalsList, key);

        var who = AiFeatureOutput.Ph(bundle.Resolution.Keys.First());
        var headline = $"{who} tracked {AiFeatureOutput.Ph(Total(AiPeriodFacts.Tracked)!)} on {AiFeatureOutput.Ph(Total(AiDayFacts.Day)!)}.";

        var sentences = new List<string>();
        if (Total(AiDayFacts.FirstStart) is { } first && Total(AiDayFacts.LastEnd) is { } last)
        {
            sentences.Add($"The day ran from {AiFeatureOutput.Ph(first)} to {AiFeatureOutput.Ph(last)}.");
        }

        var project = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.ProjectsList).Select(p => AiFeatureOutput.Value(p, "project")).FirstOrDefault();
        var app = AiFeatureOutput.ListItems(bundle, AiPeriodFacts.AppsList).Select(a => AiFeatureOutput.Value(a, "app")).FirstOrDefault();
        if (project != null)
        {
            sentences.Add(app == null
                ? $"Most time went to {AiFeatureOutput.Ph(project)}."
                : $"Most time went to {AiFeatureOutput.Ph(project)} in {AiFeatureOutput.Ph(app)}.");
        }

        var highlights = new List<AiRawHighlight?>();
        if (Total(AiDayFacts.LongestSpan) is { } span && Total(AiDayFacts.LongestSpanStart) is { } start && Total(AiDayFacts.LongestSpanEnd) is { } end)
        {
            highlights.Add(new AiRawHighlight("focus",
                $"The longest continuous stretch was {AiFeatureOutput.Ph(span)}, from {AiFeatureOutput.Ph(start)} to {AiFeatureOutput.Ph(end)}.",
                new[] { span, start, end }));
        }

        return new MemberDaySummaryRaw(headline, string.Join(" ", sentences), highlights, bundle.Caveats.Take(MaxCaveats).ToList());
    }

    public override AiMemberDaySummaryDto Render(MemberDaySummaryRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = AiFeatureOutput.RenderText(raw.Headline, bundle, AiAssistConsts.MaxHeadlineLength),
        Summary = AiFeatureOutput.RenderText(raw.Summary, bundle, AiAssistConsts.MaxSummaryLength),
        Highlights = AiFeatureOutput.RenderInsights(raw.AsInsights(), MaxHighlights, bundle),
        Caveats = AiFeatureOutput.RenderTexts(raw.Caveats, MaxCaveats, AiAssistConsts.MaxCaveatLength, bundle)
    };

    public override AiMemberDaySummaryDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = NoDataHeadline
    };

    /// <summary>3 hours while the day has not ended, 14 days for a past day (spec 15).</summary>
    public override TimeSpan CacheTtl(AiMemberDaySummaryInput input, AiContextBundle bundle) =>
        ReportCalendar.AsUtc(_clock.Now) < _dayEndUtc ? TimeSpan.FromHours(3) : TimeSpan.FromDays(14);
}

/// <summary>The day summary as the model writes it (highlights have no route).</summary>
public sealed record MemberDaySummaryRaw(
    string Headline,
    string Summary,
    IReadOnlyList<AiRawHighlight?> Highlights,
    IReadOnlyList<string?> Caveats)
{
    /// <summary>The highlights as insight items without a route, for the shared checks and rendering.</summary>
    public IReadOnlyList<AiRawInsight?> AsInsights() =>
        Highlights.Select(h => h == null ? null : new AiRawInsight(h.Kind, h.Text, h.FactRefs ?? Array.Empty<string>(), string.Empty)).ToList();
}
