using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Fakes;

/// <summary>
/// The model in tests. Every call is recorded (messages and options) and answered from a script; a call nobody
/// scripted fails the test, so a test can never reach a real provider or pass by accident. Script entries are used in
/// order, one per call; <see cref="EnqueueDelay"/> entries are waited out (honouring the call's token) and <see cref="EnqueueProbe"/>
/// entries run before the next entry answers the same call.
/// </summary>
public sealed class FakeChatClient : IChatClient
{
    public const string DefaultModel = "fake-model";

    private readonly Queue<Step> _script = new();
    private readonly List<FakeChatCall> _calls = new();
    private readonly object _lock = new();
    private int _toolCallIds;

    /// <summary>The usage reported with every scripted answer (unless an entry sets its own).</summary>
    public UsageDetails Usage { get; set; } = new() { InputTokenCount = 400, OutputTokenCount = 120, TotalTokenCount = 520 };

    public IReadOnlyList<FakeChatCall> Calls
    {
        get
        {
            lock (_lock)
            {
                return _calls.ToList();
            }
        }
    }

    /// <summary>Script entries not used yet.</summary>
    public int Pending
    {
        get
        {
            lock (_lock)
            {
                return _script.Count;
            }
        }
    }

    public FakeChatClient EnqueueJson(string json, UsageDetails? usage = null)
    {
        using (JsonDocument.Parse(json))
        {
            // Scripts must hold valid JSON; invalid output is scripted with EnqueueText.
        }

        return EnqueueText(json, usage);
    }

    public FakeChatClient EnqueueText(string text, UsageDetails? usage = null) =>
        Enqueue(new Step(_ => Answer(new ChatMessage(ChatRole.Assistant, text), ChatFinishReason.Stop, usage)));

    /// <summary>The model asks to call tool <paramref name="name"/> with <paramref name="argumentsJson"/> (an object).</summary>
    public FakeChatClient EnqueueToolCall(string name, string argumentsJson = "{}", UsageDetails? usage = null)
    {
        var arguments = JsonSerializer.Deserialize<Dictionary<string, object?>>(argumentsJson) ?? new Dictionary<string, object?>();
        return Enqueue(new Step(_ =>
        {
            var callId = "call_" + Interlocked.Increment(ref _toolCallIds);
            var message = new ChatMessage(ChatRole.Assistant, new List<AIContent> { new FunctionCallContent(callId, name, arguments) });
            return Answer(message, ChatFinishReason.ToolCalls, usage);
        }));
    }

    /// <summary>An answer that ended for <paramref name="reason"/>, such as <see cref="ChatFinishReason.Length"/> (cut
    /// off) or <see cref="ChatFinishReason.ContentFilter"/>.</summary>
    public FakeChatClient EnqueueFinishReason(ChatFinishReason reason, string text = "", UsageDetails? usage = null) =>
        Enqueue(new Step(_ => Answer(new ChatMessage(ChatRole.Assistant, text), reason, usage)));

    /// <summary>The call fails with <paramref name="exception"/>, typically an AiProviderException.</summary>
    public FakeChatClient EnqueueThrow(Exception exception) => Enqueue(new Step(_ => throw exception));

    /// <summary>The next call waits <paramref name="delay"/> (cancelled by its token) before the following entry answers it.</summary>
    public FakeChatClient EnqueueDelay(TimeSpan delay) => Enqueue(new Step(null, delay));

    /// <summary>The next call runs <paramref name="probe"/> (inside the call, to observe the caller's state) before the
    /// following entry answers it.</summary>
    public FakeChatClient EnqueueProbe(Action probe) => Enqueue(new Step(null, Probe: probe));

    public async Task<ChatResponse> GetResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        lock (_lock)
        {
            _calls.Add(new FakeChatCall(messages.ToList(), options?.Clone()));
        }

        while (true)
        {
            Step step;
            lock (_lock)
            {
                if (_script.Count == 0)
                {
                    throw new InvalidOperationException(
                        $"FakeChatClient: unscripted call #{_calls.Count}. Enqueue an answer for every model call the test expects.");
                }

                step = _script.Dequeue();
            }

            if (step.Delay is { } delay)
            {
                await Task.Delay(delay, cancellationToken);
                continue;
            }

            if (step.Probe != null)
            {
                step.Probe();
                continue;
            }

            cancellationToken.ThrowIfCancellationRequested();
            var response = step.Answer!(options);
            response.ModelId ??= options?.ModelId ?? DefaultModel;
            return response;
        }
    }

    public IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        throw new NotSupportedException("AI assist does not stream.");
    }

    public object? GetService(Type serviceType, object? serviceKey = null)
    {
        if (serviceKey != null)
        {
            return null;
        }

        if (serviceType == typeof(ChatClientMetadata))
        {
            return new ChatClientMetadata("fake", defaultModelId: DefaultModel);
        }

        return serviceType.IsInstanceOfType(this) ? this : null;
    }

    public void Dispose()
    {
    }

    private FakeChatClient Enqueue(Step step)
    {
        lock (_lock)
        {
            _script.Enqueue(step);
        }

        return this;
    }

    private ChatResponse Answer(ChatMessage message, ChatFinishReason reason, UsageDetails? usage) => new(message)
    {
        FinishReason = reason,
        Usage = usage ?? Usage
    };

    private sealed record Step(Func<ChatOptions?, ChatResponse>? Answer, TimeSpan? Delay = null, Action? Probe = null);
}

/// <summary>One call the fake received.</summary>
public sealed record FakeChatCall(IReadOnlyList<ChatMessage> Messages, ChatOptions? Options)
{
    /// <summary>Every message's text, for "the prompt contains / never contains" assertions.</summary>
    public string AllText => string.Join("\n", Messages.Select(m => m.Text));
}
