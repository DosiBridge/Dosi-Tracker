using System;
using System.ClientModel;
using System.Collections.Generic;
using System.Linq;
using System.Net.Http;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// Wraps the OpenAI adapter for every feature:
/// <list type="bullet">
/// <item>a JSON-schema response format is sent as strict structured output, so the model must follow the schema;</item>
/// <item>provider failures (after the SDK's retries) become <see cref="AiProviderException"/>: status, kind and
/// Retry-After only. The provider's message and the SDK exception are dropped, since they can echo request content or
/// account details.</item>
/// </list>
/// A cancellation the caller asked for propagates unchanged. AI assist does not stream.
/// </summary>
public sealed class OpenAiPolicyChatClient : DelegatingChatClient
{
    /// <summary>The adapter's <c>OpenAIClientExtensions.StrictKey</c> (private in Microsoft.Extensions.AI.OpenAI
    /// 10.11): <c>AdditionalProperties["strict"] = true</c> sends <c>"strict": true</c> with the json_schema format.</summary>
    public const string StrictKey = "strict";

    public OpenAiPolicyChatClient(IChatClient innerClient)
        : base(innerClient)
    {
    }

    public override async Task<ChatResponse> GetResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        try
        {
            return await base.GetResponseAsync(messages, WithStrictSchema(options), cancellationToken);
        }
        catch (Exception exception) when (cancellationToken.IsCancellationRequested && exception is not OperationCanceledException)
        {
            // The caller gave up while the SDK was still failing (for example between retries): report the
            // cancellation, not the attempts.
            throw new OperationCanceledException(cancellationToken);
        }
        catch (Exception exception) when (Translate(exception, cancellationToken) is { } translated)
        {
            throw translated;
        }
    }

    public override IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
        IEnumerable<ChatMessage> messages,
        ChatOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        throw new NotSupportedException("AI assist does not stream responses.");
    }

    /// <summary>The options with the strict flag added when they ask for a JSON schema; the caller's object is never
    /// changed.</summary>
    public static ChatOptions? WithStrictSchema(ChatOptions? options)
    {
        if (options?.ResponseFormat is not ChatResponseFormatJson { Schema: not null } ||
            options.AdditionalProperties?.ContainsKey(StrictKey) == true)
        {
            return options;
        }

        var strict = options.Clone();
        strict.AdditionalProperties = options.AdditionalProperties == null
            ? new AdditionalPropertiesDictionary()
            : new AdditionalPropertiesDictionary(options.AdditionalProperties);
        strict.AdditionalProperties[StrictKey] = true;
        return strict;
    }

    /// <summary>The <see cref="AiProviderException"/> for a provider failure, or null to let the exception propagate
    /// (a cancellation the caller asked for, or a programming error).</summary>
    public static AiProviderException? Translate(Exception exception, CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested && exception is OperationCanceledException)
        {
            return null;
        }

        switch (exception)
        {
            case AiProviderException:
                return null;
            case AggregateException aggregate:
                // The SDK's retry policy reports every failed attempt; the last one decides.
                var attempts = aggregate.Flatten().InnerExceptions;
                return attempts.Count == 0 ? null : Translate(attempts.Last(), cancellationToken);
            case ClientResultException result when result.Status > 0:
                var retryAfter = result.GetRawResponse() is { } response && response.Headers.TryGetValue("Retry-After", out var value)
                    ? AiProviderErrorMapper.ParseRetryAfterSeconds(value, DateTimeOffset.UtcNow)
                    : null;
                return new AiProviderException(AiProviderErrorMapper.Map(result.Status), result.Status, retryAfter);
            case ClientResultException noResponse:
                // No answer at all: the transport failed (DNS, TLS, connection reset) or the attempt timed out.
                return noResponse.InnerException is OperationCanceledException or TimeoutException
                    ? new AiProviderException(AiProviderErrorKind.Timeout)
                    : new AiProviderException(AiProviderErrorKind.Network);
            case HttpRequestException:
                return new AiProviderException(AiProviderErrorKind.Network);
            case OperationCanceledException or TimeoutException:
                // Not the caller's cancellation (checked above): the SDK's per-attempt network timeout.
                return new AiProviderException(AiProviderErrorKind.Timeout);
            default:
                return null;
        }
    }
}
