using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>One insight item as a card's model writes it: placeholders, fact references, an offered route id or "".</summary>
public sealed record AiRawInsight(string Kind, string Text, IReadOnlyList<string> FactRefs, string RouteId);

/// <summary>A next step as the model writes it: a short label and an offered route id.</summary>
public sealed record AiRawNextStep(string Label, string RouteId);

/// <summary>
/// The checks and the rendering the insight cards share. Only the items a card shows are checked (extra items are
/// dropped, never repaired); text is rendered from the reader's own bundle and capped after rendering.
/// </summary>
public static class AiFeatureOutput
{
    /// <summary>A text field: required when <paramref name="required"/>, then the profile's grounding rules.</summary>
    public static void ValidateText(
        List<AiViolation> violations,
        string path,
        string? text,
        AiContextBundle bundle,
        bool required = false,
        AiValidationProfile profile = AiValidationProfile.Grounded,
        IReadOnlyCollection<string>? quotedInputs = null)
    {
        if (required && AiGroundingValidator.ValidateRequired(path, text) is { } missing)
        {
            violations.Add(missing);
            return;
        }

        violations.AddRange(AiGroundingValidator.ValidateText(path, text, profile, bundle.KnownIds, quotedInputs));
    }

    /// <summary>The first <paramref name="max"/> insights: an allowed kind, grounded text, known facts, an offered
    /// route (or none).</summary>
    public static void ValidateInsights(
        List<AiViolation> violations,
        string path,
        IReadOnlyList<AiRawInsight?>? items,
        int max,
        IReadOnlyCollection<string> kinds,
        AiContextBundle bundle,
        bool withRoutes = true)
    {
        var index = 0;
        foreach (var item in AiGroundingValidator.CapList(items, max))
        {
            var at = $"{path}[{index++}]";
            if (item == null)
            {
                violations.Add(new AiViolation(at, "is required"));
                continue;
            }

            if (AiGroundingValidator.ValidateChoice(at + ".kind", item.Kind, kinds) is { } kind)
            {
                violations.Add(kind);
            }

            ValidateText(violations, at + ".text", item.Text, bundle, required: true);
            violations.AddRange(AiGroundingValidator.ValidateFactRefs(at + ".factRefs", item.FactRefs, bundle.KnownIds));
            if (withRoutes && AiGroundingValidator.ValidateRouteId(at + ".routeId", item.RouteId, bundle.Routes) is { } route)
            {
                violations.Add(route);
            }
        }
    }

    /// <summary>The first <paramref name="max"/> next steps: a grounded label and an offered route.</summary>
    public static void ValidateNextSteps(
        List<AiViolation> violations, string path, IReadOnlyList<AiRawNextStep?>? items, int max, AiContextBundle bundle)
    {
        var index = 0;
        foreach (var item in AiGroundingValidator.CapList(items, max))
        {
            var at = $"{path}[{index++}]";
            if (item == null)
            {
                violations.Add(new AiViolation(at, "is required"));
                continue;
            }

            ValidateText(violations, at + ".label", item.Label, bundle, required: true);
            if (AiGroundingValidator.ValidateRouteId(at + ".routeId", item.RouteId, bundle.Routes, allowEmpty: false) is { } route)
            {
                violations.Add(route);
            }
        }
    }

    /// <summary>The first <paramref name="max"/> plain texts (caveats, questions): grounded; blank ones are skipped.</summary>
    public static void ValidateTexts(
        List<AiViolation> violations, string path, IReadOnlyList<string?>? texts, int max, AiContextBundle bundle)
    {
        var index = 0;
        foreach (var text in AiGroundingValidator.CapList(texts, max))
        {
            ValidateText(violations, $"{path}[{index++}]", text, bundle);
        }
    }

    /// <summary>The text as the reader sees it, capped.</summary>
    public static string RenderText(string? text, AiContextBundle bundle, int maxLength) =>
        AiGroundingValidator.Cap(AiPlaceholderRenderer.Render(text, bundle), maxLength);

    public static List<AiInsightItemDto> RenderInsights(IReadOnlyList<AiRawInsight?>? items, int max, AiContextBundle bundle)
    {
        return AiGroundingValidator.CapList(items?.Where(i => i != null && !string.IsNullOrWhiteSpace(i.Text)), max)
            .Select(item => new AiInsightItemDto
            {
                Kind = item!.Kind,
                Text = RenderText(item.Text, bundle, AiAssistConsts.MaxItemLength),
                Facts = RenderFacts(item.FactRefs.Concat(AiGroundingValidator.PlaceholderIds(item.Text)), bundle),
                RouteId = string.IsNullOrEmpty(item.RouteId) ? null : item.RouteId
            })
            .ToList();
    }

    public static List<AiNextStepDto> RenderNextSteps(IReadOnlyList<AiRawNextStep?>? items, int max, AiContextBundle bundle)
    {
        return AiGroundingValidator.CapList(items?.Where(i => i != null && !string.IsNullOrEmpty(i.RouteId)), max)
            .Select(item => new AiNextStepDto
            {
                Label = RenderText(item!.Label, bundle, AiAssistConsts.MaxItemLength),
                RouteId = item.RouteId
            })
            .ToList();
    }

    /// <summary>Up to <paramref name="max"/> non-blank texts, rendered and capped.</summary>
    public static List<string> RenderTexts(IEnumerable<string?>? texts, int max, int maxLength, AiContextBundle bundle)
    {
        return AiGroundingValidator.CapList(texts?.Where(t => !string.IsNullOrWhiteSpace(t)), max)
            .Select(text => RenderText(text, bundle, maxLength))
            .ToList();
    }

    /// <summary>The facts these ids (or placeholders) name, once each in order, for "the figures used"; ids the bundle
    /// does not hold are left out.</summary>
    public static List<AiFactRefDto> RenderFacts(IEnumerable<string?> ids, AiContextBundle bundle)
    {
        var facts = new List<AiFactRefDto>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var raw in ids)
        {
            var id = raw?.Trim().Trim('{', '}');
            if (id == null || !bundle.Facts.TryGetValue(id, out var fact) || !seen.Add(id))
            {
                continue;
            }

            facts.Add(new AiFactRefDto { Id = id, Label = FactLabel(id, fact, bundle), Display = fact.Display });
        }

        return facts;
    }

    /// <summary>The fact's label, naming who or what it is about for the reader ("tracked time (Website)").</summary>
    public static string FactLabel(string id, AiFact fact, AiContextBundle bundle)
    {
        if (!bundle.FactSubjects.TryGetValue(id, out var subject))
        {
            return fact.Label;
        }

        var name = subject == AiPseudonymizer.SelfHandle
            ? AiPlaceholderRenderer.You
            : AiPlaceholderRenderer.Render("{" + subject + "}", bundle);
        return $"{fact.Label} ({name})";
    }

    /// <summary>What the bundle carries, as data categories: durations, the given ones, and the kinds of quoted text it
    /// holds (project, app and task names, window titles, notes).</summary>
    public static IReadOnlyList<string> Categories(AiContextBundle bundle, params string[] always)
    {
        var categories = new HashSet<string>(StringComparer.Ordinal) { AiDataCategories.Durations };
        categories.UnionWith(always);

        foreach (var id in bundle.Labels.Keys)
        {
            switch (id[..1])
            {
                case AiBundleWriter.ProjectPrefix:
                    categories.Add(AiDataCategories.ProjectNames);
                    break;
                case AiBundleWriter.AppPrefix:
                    categories.Add(AiDataCategories.AppNames);
                    break;
                case AiBundleWriter.TaskPrefix:
                    categories.Add(AiDataCategories.TaskTitles);
                    break;
                case AiBundleWriter.TitlePrefix:
                    categories.Add(AiDataCategories.WindowTitles);
                    break;
                case AiBundleWriter.NotePrefix:
                    categories.Add(AiDataCategories.TaskNotes);
                    break;
            }
        }

        return AiDataCategories.All.Where(categories.Contains).ToList();
    }

    /// <summary>The id at <c>lists[list][key]</c>, or null.</summary>
    public static string? ListValue(AiContextBundle bundle, string list, string key) =>
        bundle.Lists[list] is JsonObject values && values[key] is JsonValue value && value.TryGetValue<string>(out var id)
            ? id
            : null;

    /// <summary>The objects of the array <c>lists[list]</c> (empty when there is none).</summary>
    public static IReadOnlyList<JsonObject> ListItems(AiContextBundle bundle, string list) =>
        bundle.Lists[list] is JsonArray items ? items.OfType<JsonObject>().ToList() : Array.Empty<JsonObject>();

    /// <summary>The string <c>item[key]</c>, or null.</summary>
    public static string? Value(JsonObject item, string key) =>
        item[key] is JsonValue value && value.TryGetValue<string>(out var text) ? text : null;

    /// <summary>"{id}" for a placeholder.</summary>
    public static string Ph(string id) => "{" + id + "}";
}
