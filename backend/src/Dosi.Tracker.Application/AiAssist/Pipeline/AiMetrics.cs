using System;
using System.Collections.Generic;
using System.Diagnostics.Metrics;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// AI assist metrics, meter <see cref="MeterName"/> (a singleton). Tags are feature ids, outcomes and directions only:
/// never a workspace, a person, a prompt or an answer.
/// <list type="bullet">
/// <item><c>ai.requests{feature,outcome}</c>: requests served, by outcome (<see cref="AiUsageOutcomes"/>);</item>
/// <item><c>ai.tokens{direction}</c>: tokens sent (input) and received (output);</item>
/// <item><c>ai.latency_ms{feature}</c>: time spent waiting on the provider;</item>
/// <item><c>ai.injection_flags</c>: quoted texts that had an injection-shaped phrase removed.</item>
/// </list>
/// </summary>
public sealed class AiMetrics : IDisposable
{
    public const string MeterName = "Dosi.Tracker.AI";

    private readonly Counter<long> _requests;
    private readonly Counter<long> _tokens;
    private readonly Histogram<double> _latency;
    private readonly Counter<long> _injectionFlags;

    public AiMetrics()
    {
        Meter = new Meter(MeterName);
        _requests = Meter.CreateCounter<long>("ai.requests", description: "AI requests served, by feature and outcome.");
        _tokens = Meter.CreateCounter<long>("ai.tokens", unit: "{token}", description: "Tokens sent to and received from the AI provider.");
        _latency = Meter.CreateHistogram<double>("ai.latency_ms", unit: "ms", description: "Time spent waiting on the AI provider.");
        _injectionFlags = Meter.CreateCounter<long>("ai.injection_flags", description: "Quoted texts with an injection-shaped phrase removed.");
    }

    /// <summary>This instance's meter (tests listen to it alone).</summary>
    internal Meter Meter { get; }

    public void Request(string featureId, string outcome) =>
        _requests.Add(1, new KeyValuePair<string, object?>("feature", featureId), new KeyValuePair<string, object?>("outcome", outcome));

    public void Tokens(int? inputTokens, int? outputTokens)
    {
        if (inputTokens is > 0)
        {
            _tokens.Add(inputTokens.Value, new KeyValuePair<string, object?>("direction", "input"));
        }

        if (outputTokens is > 0)
        {
            _tokens.Add(outputTokens.Value, new KeyValuePair<string, object?>("direction", "output"));
        }
    }

    public void Latency(string featureId, double milliseconds) =>
        _latency.Record(milliseconds, new KeyValuePair<string, object?>("feature", featureId));

    public void InjectionFlags(int count)
    {
        if (count > 0)
        {
            _injectionFlags.Add(count);
        }
    }

    public void Dispose() => Meter.Dispose();
}
