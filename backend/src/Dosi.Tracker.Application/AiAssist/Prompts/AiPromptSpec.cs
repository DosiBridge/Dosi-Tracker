using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>
/// One feature's prompt: its versioned instructions, the task line, and the JSON schema its answer must match. The
/// version (<c>period-summary@1</c>) covers the shared policy, the instructions and the schema; it is part of the
/// result cache key and is stored in the usage ledger, so bump it on any change.
/// <para>
/// Schemas use the subset every structured-output provider accepts: <c>type</c>, <c>properties</c>, <c>required</c>
/// (naming every property), <c>additionalProperties: false</c>, <c>enum</c> and <c>items</c>. Lengths and counts are
/// not in the schema; the validator enforces them, never trusting the provider.
/// </para>
/// </summary>
public sealed class AiPromptSpec
{
    private static readonly HashSet<string> Keywords = new(StringComparer.Ordinal)
    {
        "type", "properties", "required", "additionalProperties", "enum", "items"
    };

    private static readonly HashSet<string> Types = new(StringComparer.Ordinal)
    {
        "object", "array", "string", "integer", "number", "boolean", "null"
    };

    private static readonly Regex SchemaNamePattern = new("^[A-Za-z0-9_-]{1,64}$", RegexOptions.CultureInvariant);

    /// <param name="feature">The feature the prompt belongs to.</param>
    /// <param name="version">The prompt's version number (1 and up).</param>
    /// <param name="task">The one-line task the user message opens with.</param>
    /// <param name="instructions">The feature's rules, appended to the shared policy in the system message.</param>
    /// <param name="schema">The answer's JSON schema (see the class remarks for the allowed subset).</param>
    /// <param name="schemaName">The schema's name for the provider (letters, digits, '_' and '-', at most 64).</param>
    /// <param name="maxOutputTokens">The default output limit; the feature's own when null.</param>
    /// <exception cref="ArgumentException">An argument breaks these rules, or the schema leaves the subset.</exception>
    public AiPromptSpec(
        AiFeature feature,
        int version,
        string task,
        string instructions,
        JsonElement schema,
        string schemaName,
        int? maxOutputTokens = null)
    {
        if (version < 1)
        {
            throw new ArgumentException("Prompt versions start at 1.", nameof(version));
        }

        if (string.IsNullOrWhiteSpace(task) || task.Contains('\n'))
        {
            throw new ArgumentException("The task is one non-empty line.", nameof(task));
        }

        if (string.IsNullOrWhiteSpace(instructions))
        {
            throw new ArgumentException("A prompt needs instructions.", nameof(instructions));
        }

        if (!SchemaNamePattern.IsMatch(schemaName ?? string.Empty))
        {
            throw new ArgumentException("A schema name is 1 to 64 letters, digits, '_' or '-'.", nameof(schemaName));
        }

        var problems = SchemaViolations(schema);
        if (problems.Count > 0)
        {
            throw new ArgumentException("The schema leaves the supported subset: " + string.Join("; ", problems), nameof(schema));
        }

        Feature = feature;
        Version = feature.Id + "@" + version.ToString(CultureInfo.InvariantCulture);
        Task = task.Trim();
        Instructions = instructions.ReplaceLineEndings("\n").Trim();
        Schema = schema.Clone();
        SchemaName = schemaName!;
        MaxOutputTokens = maxOutputTokens is > 0 ? maxOutputTokens.Value : feature.DefaultMaxOutputTokens;
    }

    public AiFeature Feature { get; }

    /// <summary>"{featureId}@{version}", e.g. <c>period-summary@1</c>.</summary>
    public string Version { get; }

    public string Task { get; }

    public string Instructions { get; }

    public JsonElement Schema { get; }

    public string SchemaName { get; }

    public int MaxOutputTokens { get; }

    /// <summary>A schema from its JSON text.</summary>
    public static JsonElement ParseSchema(string json)
    {
        using var document = JsonDocument.Parse(json);
        return document.RootElement.Clone();
    }

    /// <summary>Where the schema leaves the supported subset (empty when it does not). The root must be an object.</summary>
    public static IReadOnlyList<string> SchemaViolations(JsonElement schema)
    {
        var problems = new List<string>();
        if (schema.ValueKind != JsonValueKind.Object || !TypesOf(schema).Contains("object"))
        {
            problems.Add("$: the root must be an object schema");
            return problems;
        }

        Check(schema, "$", problems);
        return problems;
    }

    private static void Check(JsonElement schema, string path, List<string> problems)
    {
        if (schema.ValueKind != JsonValueKind.Object)
        {
            problems.Add($"{path}: must be a schema object");
            return;
        }

        foreach (var property in schema.EnumerateObject().Where(p => !Keywords.Contains(p.Name)))
        {
            problems.Add($"{path}: '{property.Name}' is not supported");
        }

        var types = TypesOf(schema);
        if (types.Count == 0 || types.Any(t => !Types.Contains(t)))
        {
            problems.Add($"{path}: needs a type (or types) of {string.Join(", ", Types)}");
        }

        if (types.Contains("object"))
        {
            CheckObject(schema, path, problems);
        }

        if (types.Contains("array"))
        {
            if (schema.TryGetProperty("items", out var items))
            {
                Check(items, path + "[]", problems);
            }
            else
            {
                problems.Add($"{path}: an array needs items");
            }
        }

        if (schema.TryGetProperty("enum", out var values) &&
            (values.ValueKind != JsonValueKind.Array || values.GetArrayLength() == 0 ||
             values.EnumerateArray().Any(v => v.ValueKind is not (JsonValueKind.String or JsonValueKind.Null))))
        {
            problems.Add($"{path}: enum must list strings");
        }
    }

    private static void CheckObject(JsonElement schema, string path, List<string> problems)
    {
        if (!schema.TryGetProperty("additionalProperties", out var additional) || additional.ValueKind != JsonValueKind.False)
        {
            problems.Add($"{path}: an object needs additionalProperties false");
        }

        if (!schema.TryGetProperty("properties", out var properties) || properties.ValueKind != JsonValueKind.Object)
        {
            problems.Add($"{path}: an object needs properties");
            return;
        }

        var names = properties.EnumerateObject().Select(p => p.Name).ToList();
        var required = schema.TryGetProperty("required", out var list) && list.ValueKind == JsonValueKind.Array
            ? list.EnumerateArray().Select(r => r.ValueKind == JsonValueKind.String ? r.GetString() : null).ToList()
            : null;

        if (required == null || required.Count != names.Count || !names.All(required.Contains))
        {
            problems.Add($"{path}: required must name every property");
        }

        foreach (var property in properties.EnumerateObject())
        {
            Check(property.Value, $"{path}.{property.Name}", problems);
        }
    }

    private static List<string> TypesOf(JsonElement schema)
    {
        if (!schema.TryGetProperty("type", out var type))
        {
            return new List<string>();
        }

        return type.ValueKind switch
        {
            JsonValueKind.String => new List<string> { type.GetString()! },
            JsonValueKind.Array => type.EnumerateArray().Select(t => t.ValueKind == JsonValueKind.String ? t.GetString()! : "?").ToList(),
            _ => new List<string> { "?" }
        };
    }
}
