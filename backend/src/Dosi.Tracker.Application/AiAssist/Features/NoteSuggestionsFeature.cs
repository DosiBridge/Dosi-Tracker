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
using Volo.Abp.Domain.Entities;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>One suggested note as the model writes it.</summary>
public sealed record NoteSuggestionRaw(string Text, string Basis);

/// <summary>The note suggestions as the model writes them.</summary>
public sealed record NoteSuggestionsRaw(IReadOnlyList<NoteSuggestionRaw?> Suggestions);

/// <summary>
/// "Suggest a note" (spec 11): up to 3 short notes for one block, offered only to someone who could save them (the
/// note-edit rules, plus Tracker.Activities.ViewAll for another member's block). The bundle holds the block's project,
/// its apps (at most 8) with their time, its length and how it ended; the current note and the note of the member's
/// previous block on the same project (within 2 hours) only when the workspace allows notes, its window titles (at most
/// 5) only when it allows window titles. Creative output: no people, and only digits the quoted names hold. Nothing is
/// ever written: the member saves the note they choose. A new instance serves each request.
/// </summary>
public class NoteSuggestionsFeature
    : AiInsightFeatureBase<AiNoteSuggestionsInput, NoteSuggestionsRaw, AiNoteSuggestionsDto>, ITransientDependency
{
    public const int MaxApps = 8;
    public const int MaxTitles = 5;

    public static readonly TimeSpan PreviousBlockWindow = TimeSpan.FromHours(2);

    public const string BlockList = "block";
    public const string NotesList = "notes";

    private readonly IAiReadLayer _reads;

    private AiBlock? _block;

    public NoteSuggestionsFeature(IAiReadLayer reads)
    {
        _reads = reads;
    }

    public override AiPromptSpec Prompt => NoteSuggestionsPrompt.Spec;

    public override AiValidationProfile ValidationProfile => AiValidationProfile.Creative;

    public override string? TimeZoneOf(AiNoteSuggestionsInput input) => null;

    public override bool IsRegenerate(AiNoteSuggestionsInput input) => false;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, AiNoteSuggestionsInput input)
    {
        // The rules of saving the note: 403 or 404 exactly as PUT api/app/activity/{id}/note would answer.
        _block = await _reads.NoteBlockAsync(caller, input.ActivityId);
        var audience = _block.UserId == caller.UserId ? AiUsageRecord.SelfAudience : AiUsageRecord.MemberAudience;
        return new AiAudienceScope(audience, _block.UserId, _block.ProjectId, _block.StartedAt, _block.EndedAt);
    }

    public override async Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, AiNoteSuggestionsInput input, AiAudienceScope scope)
    {
        var block = _block!;
        var writer = new AiBundleWriter(caller.UserId);
        var details = new JsonObject();

        try
        {
            var project = await _reads.ProjectAsync(caller, block.ProjectId);
            details["project"] = writer.AddProject(project.Id, project.Title);
        }
        catch (EntityNotFoundException)
        {
            // A block of a project the caller no longer sees: suggested from its apps only.
        }

        details["length"] = writer.AddFact("length of the block", DisplayFormat.Duration(block.Minutes));
        details["endReason"] = writer.AddFact("how the block ended", block.EndReason ?? "unknown");

        var apps = new JsonArray();
        foreach (var app in block.Apps.Take(MaxApps))
        {
            var id = writer.AddApp(app.App);
            apps.Add(new JsonObject
            {
                ["app"] = id,
                ["time"] = writer.AddFact("time with the app in focus",
                    app.Seconds is { } seconds ? DisplayFormat.Duration(seconds / 60.0) : DisplayFormat.NotMeasured, id)
            });
        }

        if (apps.Count > 0)
        {
            details["apps"] = apps;
        }

        writer.SetList(BlockList, details);

        if (caller.Policy.AllowWindowTitles && block.WindowTitles is { Count: > 0 })
        {
            AiDayFacts.AddTitles(writer, new[] { block }, MaxTitles);
        }

        if (caller.Policy.AllowNotes)
        {
            var notes = new JsonObject();
            if (!string.IsNullOrWhiteSpace(block.Note))
            {
                notes["current"] = writer.AddNote(block.Note);
            }

            if (await PreviousNoteAsync(caller, block) is { } previous)
            {
                notes["previous"] = writer.AddNote(previous);
            }

            if (notes.Count > 0)
            {
                writer.SetList(NotesList, notes);
            }
        }

        return writer.Build();
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) => AiFeatureOutput.Categories(bundle);

    public override IReadOnlyCollection<string> QuotedInputs(AiContextBundle bundle) => bundle.Labels.Values.ToList();

    public override IReadOnlyList<AiViolation> Validate(NoteSuggestionsRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        var quoted = QuotedInputs(bundle);
        var index = 0;
        foreach (var suggestion in AiGroundingValidator.CapList(raw.Suggestions, AiAssistConsts.MaxNoteSuggestions))
        {
            var at = $"suggestions[{index++}]";
            if (suggestion == null)
            {
                violations.Add(new AiViolation(at, "is required"));
                continue;
            }

            AiFeatureOutput.ValidateText(violations, at + ".text", suggestion.Text, bundle, required: true, ValidationProfile, quoted);
            if (AiGroundingValidator.ValidateChoice(at + ".basis", suggestion.Basis, AvailableBases(bundle)) is { } basis)
            {
                violations.Add(basis);
            }
        }

        return violations;
    }

    /// <summary>"Worked on {P1} in {A1}" and "Worked on {P1}" (spec 12.5).</summary>
    public override NoteSuggestionsRaw Fallback(AiContextBundle bundle)
    {
        var details = bundle.Lists[BlockList] as JsonObject;
        var project = details == null ? null : AiFeatureOutput.Value(details, "project");
        var app = details?["apps"] is JsonArray apps && apps.FirstOrDefault() is JsonObject first ? AiFeatureOutput.Value(first, "app") : null;

        var suggestions = new List<NoteSuggestionRaw?>();
        if (project != null && app != null)
        {
            suggestions.Add(new NoteSuggestionRaw($"Worked on {AiFeatureOutput.Ph(project)} in {AiFeatureOutput.Ph(app)}", AiNoteSuggestionBases.Apps));
        }

        if (project != null)
        {
            suggestions.Add(new NoteSuggestionRaw($"Worked on {AiFeatureOutput.Ph(project)}", AiNoteSuggestionBases.Project));
        }
        else if (app != null)
        {
            suggestions.Add(new NoteSuggestionRaw($"Worked in {AiFeatureOutput.Ph(app)}", AiNoteSuggestionBases.Apps));
        }

        return new NoteSuggestionsRaw(suggestions);
    }

    public override AiNoteSuggestionsDto Render(NoteSuggestionsRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Suggestions = AiGroundingValidator.CapList(raw.Suggestions?.Where(s => s != null && !string.IsNullOrWhiteSpace(s.Text)), AiAssistConsts.MaxNoteSuggestions)
            .Select(s => new AiNoteSuggestionDto
            {
                Text = AiFeatureOutput.RenderText(s!.Text, bundle, AiAssistConsts.MaxNoteSuggestionLength),
                Basis = s.Basis
            })
            .ToList()
    };

    public override AiNoteSuggestionsDto NoData(AiContextBundle bundle, AiResultMetaDto meta) => new() { Meta = meta };

    public override TimeSpan CacheTtl(AiNoteSuggestionsInput input, AiContextBundle bundle) => TimeSpan.FromHours(24);

    /// <summary>What a suggestion may be based on: the project and apps always, window titles and the previous note only
    /// when the bundle holds them.</summary>
    public static IReadOnlyCollection<string> AvailableBases(AiContextBundle bundle)
    {
        var bases = new List<string> { AiNoteSuggestionBases.Apps, AiNoteSuggestionBases.Project };
        if (bundle.Lists.ContainsKey(AiDayFacts.TitlesList))
        {
            bases.Add(AiNoteSuggestionBases.WindowTitles);
        }

        if (AiFeatureOutput.ListValue(bundle, NotesList, "previous") != null)
        {
            bases.Add(AiNoteSuggestionBases.PreviousNote);
        }

        return bases;
    }

    /// <summary>The note of the member's last block on the same project that ended within
    /// <see cref="PreviousBlockWindow"/> before this one started; read only for the caller's own blocks or by a caller who
    /// sees the team (the day read refuses anyone else).</summary>
    protected virtual async Task<string?> PreviousNoteAsync(AiCallerContext caller, AiBlock block)
    {
        if (block.UserId != caller.UserId && !caller.CanSeeTeam)
        {
            return null;
        }

        var start = ReportCalendar.AsUtc(block.StartedAt);
        var earlier = await _reads.DayBlocksAsync(caller, block.UserId, start - PreviousBlockWindow, start);
        return earlier
            .Where(b => b.Id != block.Id && b.ProjectId == block.ProjectId && ReportCalendar.AsUtc(b.EndedAt) >= start - PreviousBlockWindow)
            .OrderByDescending(b => b.StartedAt)
            .Select(b => b.Note)
            .FirstOrDefault(n => !string.IsNullOrWhiteSpace(n));
    }
}
