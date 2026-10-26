using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// One Ask AI tool (spec 10.2): a strict JSON schema (every property required, optional ones typed
/// <c>["string","null"]</c>, <c>additionalProperties: false</c>), bound to this turn, routed through
/// <see cref="AiToolGate"/>. A subclass reads its own arguments and builds its result; everything shared (budgets,
/// exception mapping, the citation) is <see cref="AiToolGate"/>'s job, never the tool's own.
/// </summary>
public abstract class AiTool : AIFunction
{
    protected AiTool(AiTurnContext turn, AiToolGate gate)
    {
        Turn = turn;
        Gate = gate;
    }

    protected AiTurnContext Turn { get; }

    protected AiToolGate Gate { get; }

    public sealed override string Name => ToolName;

    public sealed override string Description => ToolDescription;

    public sealed override JsonElement JsonSchema => Schema;

    protected abstract string ToolName { get; }

    protected abstract string ToolDescription { get; }

    protected abstract JsonElement Schema { get; }

    protected override ValueTask<object?> InvokeCoreAsync(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        return InvokeAsyncCore(arguments, cancellationToken);
    }

    private async ValueTask<object?> InvokeAsyncCore(AIFunctionArguments arguments, CancellationToken cancellationToken)
    {
        var result = await RunAsync(arguments, cancellationToken);
        return result;
    }

    /// <summary>Resolves the tool's own arguments and returns its JSON result through <see cref="AiToolGate.RunAsync"/>.</summary>
    protected abstract Task<JsonObject> RunAsync(AIFunctionArguments arguments, CancellationToken cancellationToken);

    /// <summary>A schema requiring every property; optional ones accept null.</summary>
    protected static JsonElement BuildSchema(params (string Name, bool Optional)[] properties)
    {
        var propertiesNode = new JsonObject();
        var required = new JsonArray();
        foreach (var (name, optional) in properties)
        {
            propertiesNode[name] = optional
                ? new JsonObject { ["type"] = new JsonArray { "string", "null" } }
                : new JsonObject { ["type"] = "string" };
            required.Add(name);
        }

        var schema = new JsonObject
        {
            ["type"] = "object",
            ["properties"] = propertiesNode,
            ["required"] = required,
            ["additionalProperties"] = false
        };

        using var document = JsonDocument.Parse(schema.ToJsonString());
        return document.RootElement.Clone();
    }

    /// <summary>A schema with no arguments.</summary>
    protected static JsonElement EmptySchema()
    {
        using var document = JsonDocument.Parse("""{"type":"object","properties":{},"required":[],"additionalProperties":false}""");
        return document.RootElement.Clone();
    }

    /// <summary>A string argument's value (null when absent, blank, or the model sent something else).</summary>
    public static string? ReadString(AIFunctionArguments arguments, string name)
    {
        if (!arguments.TryGetValue(name, out var raw) || raw == null)
        {
            return null;
        }

        var text = raw switch
        {
            JsonElement { ValueKind: JsonValueKind.String } element => element.GetString(),
            JsonElement { ValueKind: JsonValueKind.Null } => null,
            string s => s,
            _ => raw.ToString()
        };

        return string.IsNullOrWhiteSpace(text) ? null : text.Trim();
    }

    /// <summary>The argument's value, or <paramref name="fallback"/> when it is absent.</summary>
    public static string? ReadStringOrDefault(AIFunctionArguments arguments, string name, string? fallback) =>
        ReadString(arguments, name) ?? fallback;

    /// <summary>Common query arguments every reporting tool takes: a named period or an explicit custom range, a
    /// project ref, and a member handle.</summary>
    protected static readonly IReadOnlyList<(string Name, bool Optional)> PeriodProjectMemberArgs = new[]
    {
        ("period", true), ("from", true), ("to", true), ("project", true), ("member", true)
    };
}
