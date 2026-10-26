using System;
using System.ClientModel.Primitives;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The OpenAI SDK's retry policy (exponential back-off with jitter, honouring Retry-After, at most
/// <c>AI:MaxRetries</c> retries), widened to every 5xx: the SDK retries only 408, 429, 500 and 502-504, and OpenAI's
/// edge answers 520-529 intermittently. A Retry-After longer than <see cref="MaxHonouredRetryAfter"/> is not waited
/// for: the call fails at once with the provider's delay, so the caller can say when to try again instead of holding
/// the request open.
/// </summary>
public class OpenAiRetryPolicy : ClientRetryPolicy
{
    public static readonly TimeSpan MaxHonouredRetryAfter = TimeSpan.FromSeconds(10);

    public OpenAiRetryPolicy(int maxRetries)
        : base(maxRetries)
    {
        MaxRetries = maxRetries;
    }

    public int MaxRetries { get; }

    protected override void OnSendingRequest(PipelineMessage message)
    {
        Classify(message);
        base.OnSendingRequest(message);
    }

    protected override ValueTask OnSendingRequestAsync(PipelineMessage message)
    {
        Classify(message);
        return base.OnSendingRequestAsync(message);
    }

    private static void Classify(PipelineMessage message)
    {
        if (message.ResponseClassifier is not TransientServerErrorClassifier)
        {
            message.ResponseClassifier = new TransientServerErrorClassifier(message.ResponseClassifier);
        }
    }

    /// <summary>The SDK's own classification (which statuses are errors), with 5xx retriable and long Retry-After
    /// waits refused.</summary>
    private sealed class TransientServerErrorClassifier : PipelineMessageClassifier
    {
        private readonly PipelineMessageClassifier _inner;

        public TransientServerErrorClassifier(PipelineMessageClassifier inner)
        {
            _inner = inner;
        }

        public override bool TryClassify(PipelineMessage message, out bool isError) => _inner.TryClassify(message, out isError);

        public override bool TryClassify(PipelineMessage message, Exception? exception, out bool isRetriable)
        {
            var response = message.Response;
            if (exception == null && response != null && AiProviderErrorMapper.IsRetriableStatus(response.Status))
            {
                isRetriable = !AsksForLongWait(response);
                return true;
            }

            return _inner.TryClassify(message, exception, out isRetriable);
        }

        private static bool AsksForLongWait(PipelineResponse response)
        {
            return response.Headers.TryGetValue("Retry-After", out var value) &&
                   AiProviderErrorMapper.ParseRetryAfterSeconds(value, DateTimeOffset.UtcNow) is { } seconds &&
                   seconds > MaxHonouredRetryAfter.TotalSeconds;
        }
    }
}
