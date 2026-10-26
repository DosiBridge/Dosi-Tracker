using System;
using System.Collections.Generic;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Microsoft.Extensions.AI;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary>Gives each request the random tag its data blocks are delimited with (replaced in tests).</summary>
public interface INonceSource
{
    /// <summary>A fresh nonce: <see cref="AiPromptBuilder.NonceLength"/> lowercase hex characters.</summary>
    string Next();
}

/// <summary>Nonces from the operating system's cryptographic random generator.</summary>
public class RandomNonceSource : INonceSource, ISingletonDependency
{
    public string Next() => Convert.ToHexStringLower(RandomNumberGenerator.GetBytes(AiPromptBuilder.NonceLength / 2));
}

/// <summary>
/// Assembles a card's messages:
/// <code>
/// [system] shared policy (for the audience) + the feature's instructions [+ the response schema]
/// [user]   TASK: …
///          &lt;data-{nonce}&gt; canonical JSON of the bundle &lt;/data-{nonce}&gt;
///          &lt;untrusted-{nonce}&gt; JSON of id → sanitized quoted text &lt;/untrusted-{nonce}&gt;
///          Write the JSON object now.
/// </code>
/// The static part comes first so providers can cache it. The nonce is fresh per request and never part of the
/// cache key: quoted text cannot guess it to close a block early (and cannot write a tag at all, see
/// <see cref="UntrustedText"/>).
/// </summary>
public class AiPromptBuilder : ITransientDependency
{
    public const int NonceLength = 8;
    public const int MaxRepairViolations = 5;

    public const string WriteJsonLine = "Write the JSON object now.";
    public const string RepairJsonLine = "Return the corrected JSON object only.";
    public const string RepairTextLine = "Return the corrected answer only.";

    private static readonly Regex NoncePattern = new("^[0-9a-f]{8}$", RegexOptions.CultureInvariant);

    private readonly INonceSource _nonceSource;

    public AiPromptBuilder(INonceSource nonceSource)
    {
        _nonceSource = nonceSource;
    }

    /// <summary>The messages for this request, with a fresh nonce.</summary>
    public virtual IList<ChatMessage> Build(AiPromptSpec spec, AiContextBundle bundle, string audience, bool includeSchema = false)
    {
        return Build(spec, bundle, audience, _nonceSource.Next(), includeSchema);
    }

    /// <param name="spec">The feature's prompt.</param>
    /// <param name="bundle">What the model may use.</param>
    /// <param name="audience">Who reads the answer (see <see cref="AiSharedPolicy.Audiences"/>).</param>
    /// <param name="nonce">The block tag: <see cref="NonceLength"/> lowercase hex characters.</param>
    /// <param name="includeSchema">Also write the schema into the system message, for providers that only take
    /// "a JSON object" as the response format.</param>
    public static IList<ChatMessage> Build(AiPromptSpec spec, AiContextBundle bundle, string audience, string nonce, bool includeSchema = false)
    {
        if (!NoncePattern.IsMatch(nonce ?? string.Empty))
        {
            throw new ArgumentException($"A nonce is {NonceLength} lowercase hex characters.", nameof(nonce));
        }

        var system = new StringBuilder()
            .Append(AiSharedPolicy.For(audience))
            .Append("\n\n")
            .Append(spec.Instructions);

        if (includeSchema)
        {
            system.Append("\n\nResponse schema (")
                .Append(spec.SchemaName)
                .Append("):\n")
                .Append(AiCanonicalJson.Serialize(JsonNode.Parse(spec.Schema.GetRawText())));
        }

        var user = new StringBuilder()
            .Append("TASK: ").Append(spec.Task).Append('\n')
            .Append("<data-").Append(nonce).Append(">\n")
            .Append(AiCanonicalJson.Data(bundle)).Append('\n')
            .Append("</data-").Append(nonce).Append(">\n")
            .Append("<untrusted-").Append(nonce).Append(">\n")
            .Append(AiCanonicalJson.Labels(bundle)).Append('\n')
            .Append("</untrusted-").Append(nonce).Append(">\n")
            .Append(WriteJsonLine);

        return new List<ChatMessage>
        {
            new(ChatRole.System, system.ToString()),
            new(ChatRole.User, user.ToString())
        };
    }

    /// <summary>
    /// The conversation for the one repair turn: the earlier messages, the model's previous answer, and a message
    /// listing (up to <see cref="MaxRepairViolations"/>) broken rules. The messages are copied, never changed.
    /// </summary>
    /// <param name="jsonAnswer">False for plain-text answers (Ask AI).</param>
    public static IList<ChatMessage> BuildRepair(
        IEnumerable<ChatMessage> messages, string? previousText, IReadOnlyCollection<AiViolation> violations, bool jsonAnswer = true)
    {
        var repair = new StringBuilder("Your previous answer broke these rules:");
        foreach (var violation in violations.Take(MaxRepairViolations))
        {
            repair.Append("\n- ").Append(violation.Path).Append(": ").Append(violation.Message);
        }

        repair.Append('\n').Append(jsonAnswer ? RepairJsonLine : RepairTextLine);

        return messages
            .Append(new ChatMessage(ChatRole.Assistant, previousText ?? string.Empty))
            .Append(new ChatMessage(ChatRole.User, repair.ToString()))
            .ToList();
    }
}
