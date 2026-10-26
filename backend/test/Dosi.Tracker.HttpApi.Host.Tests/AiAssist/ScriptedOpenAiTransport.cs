using System;
using System.Collections.Generic;
using System.Net;
using System.Net.Http;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The OpenAI SDK's HTTP layer in tests: answers only the responses a test scripted, and fails any other request, so
/// no test can reach a provider. Records every request it saw.
/// </summary>
public sealed class ScriptedOpenAiTransport : HttpMessageHandler
{
    private readonly Queue<Func<CancellationToken, Task<HttpResponseMessage>>> _steps = new();
    private readonly object _lock = new();

    public List<RecordedRequest> Requests { get; } = new();

    public sealed record RecordedRequest(HttpMethod Method, Uri Uri, IReadOnlyDictionary<string, string> Headers, string Body);

    /// <summary>A chat completion whose assistant message is <paramref name="content"/>.</summary>
    public ScriptedOpenAiTransport Completion(string content = "{\"ok\":true}", string finishReason = "stop")
    {
        var body = System.Text.Json.JsonSerializer.Serialize(new
        {
            id = "chatcmpl-test",
            @object = "chat.completion",
            created = 1767225600,
            model = "gpt-5.4-mini-2026-03-17",
            choices = new[]
            {
                new { index = 0, message = new { role = "assistant", content }, finish_reason = finishReason }
            },
            usage = new
            {
                prompt_tokens = 120,
                completion_tokens = 30,
                total_tokens = 150,
                prompt_tokens_details = new { cached_tokens = 20 },
                completion_tokens_details = new { reasoning_tokens = 10 }
            }
        });
        return Status(200, body);
    }

    /// <summary>An error answer whose body carries provider text that must never surface.</summary>
    public ScriptedOpenAiTransport Status(int status, string? body = null, string? retryAfter = null)
    {
        body ??= """{"error":{"message":"PROVIDER-DETAIL account org-test quota","type":"server_error","code":null}}""";
        return Enqueue(_ =>
        {
            var response = new HttpResponseMessage((HttpStatusCode)status)
            {
                Content = new StringContent(body, Encoding.UTF8, "application/json")
            };
            if (retryAfter != null)
            {
                response.Headers.TryAddWithoutValidation("Retry-After", retryAfter);
            }

            return Task.FromResult(response);
        });
    }

    public ScriptedOpenAiTransport NetworkFailure() =>
        Enqueue(_ => throw new HttpRequestException("PROVIDER-DETAIL connection reset"));

    /// <summary>Never answers: the request ends only by the SDK's timeout or the caller's cancellation.</summary>
    public ScriptedOpenAiTransport Hang() => Enqueue(async ct =>
    {
        await Task.Delay(Timeout.InfiniteTimeSpan, ct);
        throw new InvalidOperationException("unreachable");
    });

    private ScriptedOpenAiTransport Enqueue(Func<CancellationToken, Task<HttpResponseMessage>> step)
    {
        lock (_lock)
        {
            _steps.Enqueue(step);
        }

        return this;
    }

    protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
    {
        Func<CancellationToken, Task<HttpResponseMessage>> step;
        lock (_lock)
        {
            var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (var header in request.Headers)
            {
                headers[header.Key] = string.Join(",", header.Value);
            }

            var body = request.Content == null ? "" : request.Content.ReadAsStringAsync(cancellationToken).GetAwaiter().GetResult();
            Requests.Add(new RecordedRequest(request.Method, request.RequestUri!, headers, body));

            if (_steps.Count == 0)
            {
                throw new InvalidOperationException($"Unscripted provider request {request.Method} {request.RequestUri}.");
            }

            step = _steps.Dequeue();
        }

        return await step(cancellationToken);
    }
}

/// <summary>Collects log lines (level and rendered text) for assertions.</summary>
public sealed class ListLogger : ILogger
{
    public List<(LogLevel Level, string Message)> Entries { get; } = new();

    public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => true;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception,
        Func<TState, Exception?, string> formatter)
    {
        lock (Entries)
        {
            Entries.Add((logLevel, formatter(state, exception) + (exception == null ? "" : " " + exception)));
        }
    }
}
