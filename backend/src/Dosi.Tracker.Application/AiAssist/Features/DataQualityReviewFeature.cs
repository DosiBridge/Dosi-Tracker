using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Read;
using Dosi.Tracker.Reporting;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Timing;
using Volo.Abp.Validation;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>One item of a data-quality review as the model writes it.</summary>
public sealed record DataQualityRawItem(
    string CandidateId,
    string Title,
    string Explanation,
    IReadOnlyList<string?> PossibleReasons,
    string ActionId);

/// <summary>The data-quality review as the model writes it.</summary>
public sealed record DataQualityReviewRaw(IReadOnlyList<DataQualityRawItem?> Items);

/// <summary>
/// "Week review" and "Agent health" (spec 11): <see cref="DataQualityRules"/> find the candidates (at most 20), the
/// model picks and explains at most 5. Team scope needs a caller who sees the team; self scope reads the caller's own
/// data. Without candidates nothing is sent. An item naming an unknown candidate is dropped, as is an action the
/// candidate does not allow; kind, severity, member and figures always come from the candidate, never from the model.
/// A new instance serves each request.
/// </summary>
public class DataQualityReviewFeature
    : AiInsightFeatureBase<AiDataQualityReviewInput, DataQualityReviewRaw, AiDataQualityReviewDto>, ITransientDependency
{
    public const int MaxItems = 5;
    public const int MaxReasons = 3;
    public const int MaxTitleLength = 120;
    public const int MaxReasonLength = 160;

    public const string CandidatePrefix = "C";

    private readonly IAiReadLayer _reads;
    private readonly AiScopeGuard _scopeGuard;
    private readonly IClock _clock;

    private bool _team;
    private IReadOnlyCollection<string> _kinds = Array.Empty<string>();

    public DataQualityReviewFeature(IAiReadLayer reads, AiScopeGuard scopeGuard, IClock clock)
    {
        _reads = reads;
        _scopeGuard = scopeGuard;
        _clock = clock;
    }

    public override AiPromptSpec Prompt => DataQualityReviewPrompt.Spec;

    public override string? TimeZoneOf(AiDataQualityReviewInput input) => input.TimeZone;

    public override bool IsRegenerate(AiDataQualityReviewInput input) => input.Regenerate;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiDataQualityReviewInput input)
    {
        _team = AiFeatureScopeInput.IsTeam(input.Scope);
        if (_team)
        {
            _scopeGuard.EnsureTeamScope(caller);
        }

        var from = ReportCalendar.AsUtc(input.From);
        var to = ReportCalendar.AsUtc(input.To);
        _scopeGuard.EnsureRange(from, to, AiAssistConsts.MaxPeriodDays);
        _kinds = ResolveKinds(input.Kinds, _team);

        var project = input.ProjectId.HasValue ? await _reads.ProjectAsync(caller, input.ProjectId.Value) : null;
        return new AiAudienceScope(
            _team ? AiUsageRecord.TeamAudience : AiUsageRecord.SelfAudience, _team ? null : caller.UserId, project?.Id, from, to);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiDataQualityReviewInput input, AiAudienceScope scope)
    {
        var from = scope.RangeFrom!.Value;
        var to = scope.RangeTo!.Value;
        var query = new AiQuery(from, to, scope.ProjectId, _team ? null : caller.UserId);

        var memberDays = _kinds.Contains(AiDataQualityKinds.DataGap) ? await _reads.UserDailyAsync(caller, query) : Array.Empty<AiMemberDayRow>();
        var quality = await _reads.DataQualityAsync(caller, query);
        var roster = await _reads.RosterAsync(caller);
        // Never-tracked asks about the workspace, so it is not looked for in a project's review.
        var lookForNeverTracked = _team && scope.ProjectId == null && _kinds.Contains(AiDataQualityKinds.NeverTracked);
        var recent = lookForNeverTracked
            ? await _reads.DataQualityAsync(caller, query with { From = to.AddDays(-DataQualityRules.NeverTrackedWindowDays) })
            : Array.Empty<AiDataQualityRow>();
        var devices = _team && _kinds.Contains(AiDataQualityKinds.OutdatedAgent) ? await _reads.DevicesAsync(caller, query) : Array.Empty<AiDeviceRow>();

        var candidates = DataQualityRules.Find(new DataQualityContext(
            ReportCalendar.ResolveZone(caller.TimeZone), from, to, _clock.Now, _team, caller.UserId,
            lookForNeverTracked ? _kinds : _kinds.Where(k => k != AiDataQualityKinds.NeverTracked).ToList(),
            roster, memberDays, quality, recent, devices));
        if (candidates.Count == 0)
        {
            return new AiBundleWriter(caller.UserId).Build();
        }

        var writer = new AiBundleWriter(AiPseudonymizer.ForCard(
            caller.UserId, candidates.Select(c => new AiMemberWeight(c.UserId, c.Weight)), DataQualityRules.MaxCandidates));
        var number = 0;
        foreach (var candidate in candidates)
        {
            var handle = writer.AddMember(candidate.UserId, candidate.DisplayName);
            var facts = new JsonObject();
            foreach (var fact in candidate.Facts)
            {
                facts[fact.Key] = writer.AddFact(fact.Label, fact.Display, handle);
            }

            writer.AddCandidate(new JsonObject
            {
                ["id"] = CandidatePrefix + (++number).ToString(System.Globalization.CultureInfo.InvariantCulture),
                ["kind"] = candidate.Kind,
                ["severity"] = candidate.Severity,
                ["member"] = handle,
                ["facts"] = facts,
                ["allowedActions"] = new JsonArray(candidate.Actions.Select(a => (JsonNode?)a.Id).ToArray())
            });

            foreach (var action in candidate.Actions.Where(a => a.RouteId != null))
            {
                writer.AddRoute(action.RouteId!);
            }
        }

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        AiFeatureOutput.Categories(bundle, AiDataCategories.Times, AiDataCategories.ActivityPercent, AiDataCategories.AgentInfo);

    public override IReadOnlyList<AiViolation> Validate(DataQualityReviewRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        var index = 0;
        foreach (var item in Kept(raw, bundle))
        {
            var at = $"items[{index++}]";
            AiFeatureOutput.ValidateText(violations, at + ".title", item.Title, bundle, required: true);
            AiFeatureOutput.ValidateText(violations, at + ".explanation", item.Explanation, bundle, required: true);
            AiFeatureOutput.ValidateTexts(violations, at + ".possibleReasons", item.PossibleReasons, MaxReasons, bundle);
        }

        return violations;
    }

    /// <summary>One templated item per candidate, notices first (spec 12.5).</summary>
    public override DataQualityReviewRaw Fallback(AiContextBundle bundle)
    {
        var items = Candidates(bundle)
            .Select((c, order) => (Candidate: c, Order: order))
            .OrderBy(c => DataQualityRules.SeverityRank(c.Candidate.Severity))
            .ThenBy(c => c.Order)
            .Take(MaxItems)
            .Select(c => Template(c.Candidate))
            .ToList();
        return new DataQualityReviewRaw(items);
    }

    public override AiDataQualityReviewDto Render(DataQualityReviewRaw raw, AiContextBundle bundle, AiResultMetaDto meta)
    {
        var candidates = Candidates(bundle).ToDictionary(c => c.Id, StringComparer.Ordinal);
        return new AiDataQualityReviewDto
        {
            Meta = meta,
            CandidateCount = candidates.Count,
            Items = Kept(raw, bundle).Select(item =>
            {
                var candidate = candidates[item.CandidateId];
                var action = DataQualityRules.Actions[candidate.Kind].FirstOrDefault(a => a.Id == item.ActionId);
                return new AiDataQualityItemDto
                {
                    CandidateId = candidate.Id,
                    Kind = candidate.Kind,
                    Severity = candidate.Severity,
                    Title = AiFeatureOutput.RenderText(item.Title, bundle, MaxTitleLength),
                    Explanation = AiFeatureOutput.RenderText(item.Explanation, bundle, AiAssistConsts.MaxItemLength),
                    PossibleReasons = AiFeatureOutput.RenderTexts(item.PossibleReasons, MaxReasons, MaxReasonLength, bundle),
                    ActionId = action?.Id,
                    ActionRouteId = action?.RouteId,
                    SubjectName = candidate.Member == null ? null
                        : candidate.Member == AiPseudonymizer.SelfHandle ? AiPlaceholderRenderer.You
                        : AiPlaceholderRenderer.Render(AiFeatureOutput.Ph(candidate.Member), bundle),
                    SubjectUserId = candidate.Member != null && bundle.Resolution.TryGetValue(candidate.Member, out var alias) ? alias.UserId : null,
                    Facts = AiFeatureOutput.RenderFacts(candidate.Facts.Values, bundle)
                };
            }).ToList()
        };
    }

    public override AiDataQualityReviewDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new() { Meta = meta };

    public override TimeSpan CacheTtl(AiDataQualityReviewInput input, AiContextBundle bundle) => TimeSpan.FromHours(12);

    /// <summary>The kinds to look for: the requested ones (each known, and team-only ones only in a team review), or every
    /// kind the scope allows.</summary>
    /// <exception cref="AbpValidationException">An unknown kind, or a team-only kind in a self review.</exception>
    public static IReadOnlyCollection<string> ResolveKinds(IReadOnlyCollection<string>? requested, bool team)
    {
        var allowed = team ? AiDataQualityKinds.All : AiDataQualityKinds.Self;
        if (requested == null || requested.Count == 0)
        {
            return allowed;
        }

        var kinds = requested.Select(k => k?.Trim() ?? string.Empty).Distinct(StringComparer.Ordinal).ToList();
        if (kinds.Any(k => !allowed.Contains(k)))
        {
            throw new AbpValidationException(
                "The kinds are not valid.",
                new List<ValidationResult> { new("Choose from: " + string.Join(", ", allowed) + ".", new[] { "kinds" }) });
        }

        return kinds;
    }

    /// <summary>The model's items that name a candidate, once each, at most <see cref="MaxItems"/>.</summary>
    private static IEnumerable<DataQualityRawItem> Kept(DataQualityReviewRaw raw, AiContextBundle bundle)
    {
        var known = Candidates(bundle).Select(c => c.Id).ToHashSet(StringComparer.Ordinal);
        var seen = new HashSet<string>(StringComparer.Ordinal);
        return (raw.Items ?? Array.Empty<DataQualityRawItem?>())
            .Where(i => i != null && i.CandidateId != null && known.Contains(i.CandidateId) && seen.Add(i.CandidateId))
            .Take(MaxItems)
            .Select(i => i!);
    }

    private static DataQualityRawItem Template(BundleCandidate c)
    {
        string F(string key) => AiFeatureOutput.Ph(c.Facts[key]);
        var who = AiFeatureOutput.Ph(c.Member ?? AiPseudonymizer.SelfHandle);
        var action = DataQualityRules.Actions[c.Kind].FirstOrDefault()?.Id ?? string.Empty;

        return c.Kind switch
        {
            AiDataQualityKinds.DataGap => new DataQualityRawItem(c.Id, "Weekdays without tracked time",
                $"No time was tracked for {who} from {F("first")} to {F("last")}, {F("weekdays")} weekdays in a row.",
                new[] { "Leave or days off", "The agent was not running", "Work that is not tracked" }, action),
            AiDataQualityKinds.ClockSkew => new DataQualityRawItem(c.Id, "A device clock is off",
                $"A device used by {who} reported a clock difference of up to {F("skew")}.",
                new[] { "The clock is set by hand", "Automatic time sync is off" }, action),
            AiDataQualityKinds.UnmeasuredActivity => new DataQualityRawItem(c.Id, "Activity was not measured",
                $"Activity % was not measured for {F("share")} of the time tracked by {who}.",
                new[] { "Keyboard and mouse tracking is off for the project", "The operating system did not grant input access", "A session where input cannot be read" },
                action),
            AiDataQualityKinds.SoftwareInput => new DataQualityRawItem(c.Id, "Input from software",
                $"{F("blocks")} blocks tracked by {who} had input generated by software.",
                new[] { "Remote desktop", "Accessibility tools", "Automation tools" }, action),
            AiDataQualityKinds.NeverTracked => new DataQualityRawItem(c.Id, "No tracked time yet",
                $"{who} joined on {F("joined")} and has no tracked time in the last {F("window")}.",
                new[] { "The agent is not installed yet", "The role does not involve tracking" }, action),
            _ => new DataQualityRawItem(c.Id, "Agent update available",
                $"The {F("platform")} agent used by {who} is version {F("version")}; the newest seen is {F("newest")}.",
                new[] { "Automatic updates are off", "The device was offline" }, action)
        };
    }

    /// <summary>A candidate as the bundle holds it.</summary>
    private sealed record BundleCandidate(string Id, string Kind, string Severity, string? Member, IReadOnlyDictionary<string, string> Facts);

    private static IReadOnlyList<BundleCandidate> Candidates(AiContextBundle bundle)
    {
        return bundle.Candidates.OfType<JsonObject>()
            .Select(c => new BundleCandidate(
                AiFeatureOutput.Value(c, "id")!,
                AiFeatureOutput.Value(c, "kind")!,
                AiFeatureOutput.Value(c, "severity")!,
                AiFeatureOutput.Value(c, "member"),
                c["facts"] is JsonObject facts
                    ? facts.Where(f => f.Value is JsonValue).ToDictionary(f => f.Key, f => (string)f.Value!, StringComparer.Ordinal)
                    : new Dictionary<string, string>(StringComparer.Ordinal)))
            .ToList();
    }
}
