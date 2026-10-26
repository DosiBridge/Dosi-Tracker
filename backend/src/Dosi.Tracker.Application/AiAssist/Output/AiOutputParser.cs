using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>What <see cref="AiOutputParser"/> made of a model's answer: the value, or why there is none.</summary>
public sealed class AiParseResult<T>
    where T : class
{
    internal AiParseResult(T? value, string text, IReadOnlyList<AiViolation> violations)
    {
        Value = value;
        Text = text;
        Violations = violations;
    }

    /// <summary>The parsed answer; null when <see cref="Violations"/> is not empty.</summary>
    public T? Value { get; }

    /// <summary>The raw answer text (for the repair turn; never logged or returned to callers).</summary>
    public string Text { get; }

    public IReadOnlyList<AiViolation> Violations { get; }

    public bool Succeeded => Value != null && Violations.Count == 0;
}

/// <summary>
/// Reads a card's JSON answer strictly: exactly one JSON object of the expected shape, with no member the shape does
/// not name, nothing before or after it, and an answer the provider did not cut off or filter.
/// </summary>
public static class AiOutputParser
{
    // Raw answer types are read with camelCase names (as in the schemas), unknown members refused, and required
    // constructor parameters and non-nullable references enforced.
    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        PropertyNameCaseInsensitive = false,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        RespectNullableAnnotations = true,
        RespectRequiredConstructorParameters = true,
        AllowTrailingCommas = false,
        ReadCommentHandling = JsonCommentHandling.Disallow,
        NumberHandling = JsonNumberHandling.Strict,
        MaxDepth = 16
    };

    /// <summary>Parses the response's text, checking its finish reason first.</summary>
    public static AiParseResult<T> Parse<T>(ChatResponse response)
        where T : class
    {
        return Parse<T>(response.Text, response.FinishReason);
    }

    /// <summary>Parses <paramref name="text"/>; a <see cref="ChatFinishReason.Length"/> or
    /// <see cref="ChatFinishReason.ContentFilter"/> finish is a violation whatever the text.</summary>
    public static AiParseResult<T> Parse<T>(string? text, ChatFinishReason? finishReason)
        where T : class
    {
        var raw = text ?? string.Empty;

        if (finishReason == ChatFinishReason.Length)
        {
            return Failed<T>(raw, "the answer was cut off at the output limit; keep it shorter");
        }

        if (finishReason == ChatFinishReason.ContentFilter)
        {
            return Failed<T>(raw, "the answer was withheld by the provider's content filter");
        }

        var trimmed = raw.Trim();
        if (trimmed.Length == 0 || trimmed[0] != '{')
        {
            return Failed<T>(raw, "the answer must be exactly one JSON object and nothing else");
        }

        try
        {
            var value = JsonSerializer.Deserialize<T>(trimmed, Options);
            return value == null
                ? Failed<T>(raw, "the answer must be exactly one JSON object and nothing else")
                : new AiParseResult<T>(value, raw, Array.Empty<AiViolation>());
        }
        catch (JsonException e)
        {
            // The exception's message can quote the answer; only the JSON path is kept, reduced to plain path characters.
            return Failed<T>(raw, "the answer does not match the response schema", SafePath(e.Path));
        }
    }

    private static string SafePath(string? path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return "$";
        }

        // Member names that need quoting ("$['…']") are the answer's own text: the path stops before them.
        var quoted = path.IndexOf("['", StringComparison.Ordinal);
        var plain = quoted < 0 ? path : path[..quoted];
        var safe = new string(plain.Where(c => char.IsAsciiLetterOrDigit(c) || c is '$' or '.' or '[' or ']' or '_').Take(64).ToArray());
        return safe.Length == 0 ? "$" : safe;
    }

    private static AiParseResult<T> Failed<T>(string raw, string message, string path = "$")
        where T : class
    {
        return new AiParseResult<T>(null, raw, new[] { new AiViolation(path, message) });
    }
}
