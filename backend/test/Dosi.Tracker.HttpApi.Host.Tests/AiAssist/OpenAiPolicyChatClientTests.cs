using System;
using System.ClientModel;
using System.Net.Http;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>OpenAiPolicyChatClient's own rules: when the strict flag is added (without touching the caller's options),
/// and which exceptions become AiProviderException.</summary>
public class OpenAiPolicyChatClientTests
{
    private static ChatResponseFormat Schema() => ChatResponseFormat.ForJsonSchema(
        JsonDocument.Parse("""{"type":"object"}""").RootElement, "card");

    [Fact]
    public void A_Schema_Format_Gets_The_Strict_Flag_On_A_Copy()
    {
        var options = new ChatOptions
        {
            ResponseFormat = Schema(),
            MaxOutputTokens = 200,
            AdditionalProperties = new AdditionalPropertiesDictionary { ["other"] = 1 }
        };

        var strict = OpenAiPolicyChatClient.WithStrictSchema(options)!;

        strict.ShouldNotBeSameAs(options);
        strict.MaxOutputTokens.ShouldBe(200);
        strict.AdditionalProperties![OpenAiPolicyChatClient.StrictKey].ShouldBe(true);
        strict.AdditionalProperties["other"].ShouldBe(1);
        options.AdditionalProperties.ContainsKey(OpenAiPolicyChatClient.StrictKey).ShouldBeFalse();
    }

    [Fact]
    public void Other_Formats_And_An_Explicit_Choice_Are_Left_Alone()
    {
        OpenAiPolicyChatClient.WithStrictSchema(null).ShouldBeNull();

        var text = new ChatOptions { ResponseFormat = ChatResponseFormat.Text };
        OpenAiPolicyChatClient.WithStrictSchema(text).ShouldBeSameAs(text);

        var jsonObject = new ChatOptions { ResponseFormat = ChatResponseFormat.Json };
        OpenAiPolicyChatClient.WithStrictSchema(jsonObject).ShouldBeSameAs(jsonObject);

        var explicitChoice = new ChatOptions
        {
            ResponseFormat = Schema(),
            AdditionalProperties = new AdditionalPropertiesDictionary { [OpenAiPolicyChatClient.StrictKey] = false }
        };
        OpenAiPolicyChatClient.WithStrictSchema(explicitChoice).ShouldBeSameAs(explicitChoice);
    }

    [Fact]
    public void Transport_Failures_Become_Provider_Errors()
    {
        OpenAiPolicyChatClient.Translate(new HttpRequestException("x"), CancellationToken.None)!.Kind
            .ShouldBe(AiProviderErrorKind.Network);
        OpenAiPolicyChatClient.Translate(new TaskCanceledException(), CancellationToken.None)!.Kind
            .ShouldBe(AiProviderErrorKind.Timeout);
        OpenAiPolicyChatClient.Translate(new TimeoutException(), CancellationToken.None)!.Kind
            .ShouldBe(AiProviderErrorKind.Timeout);
        OpenAiPolicyChatClient.Translate(new ClientResultException("x", null, new HttpRequestException("y")), CancellationToken.None)!
            .Kind.ShouldBe(AiProviderErrorKind.Network);
        OpenAiPolicyChatClient.Translate(new ClientResultException("x", null, new TaskCanceledException()), CancellationToken.None)!
            .Kind.ShouldBe(AiProviderErrorKind.Timeout);
    }

    [Fact]
    public void The_Last_Attempt_Of_An_Aggregate_Decides()
    {
        var aggregate = new AggregateException(new HttpRequestException("first"), new TaskCanceledException());

        OpenAiPolicyChatClient.Translate(aggregate, CancellationToken.None)!.Kind.ShouldBe(AiProviderErrorKind.Timeout);
        OpenAiPolicyChatClient.Translate(new AggregateException(), CancellationToken.None).ShouldBeNull();
    }

    [Fact]
    public void Cancellations_The_Caller_Asked_For_And_Programming_Errors_Propagate()
    {
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();

        OpenAiPolicyChatClient.Translate(new OperationCanceledException(cancelled.Token), cancelled.Token).ShouldBeNull();
        OpenAiPolicyChatClient.Translate(new InvalidOperationException(), CancellationToken.None).ShouldBeNull();
        OpenAiPolicyChatClient.Translate(new AiProviderException(AiProviderErrorKind.Auth, 401), CancellationToken.None).ShouldBeNull();
    }

    [Fact]
    public async Task A_Failure_After_The_Callers_Cancellation_Is_Reported_As_Cancelled()
    {
        using var cancellation = new CancellationTokenSource();
        var client = new OpenAiPolicyChatClient(new ThrowingChatClient(() =>
        {
            cancellation.Cancel();
            return new AggregateException(new HttpRequestException("attempt"));
        }));

        await Should.ThrowAsync<OperationCanceledException>(
            () => client.GetResponseAsync("hi", cancellationToken: cancellation.Token));
    }

    [Fact]
    public void Streaming_Is_Not_Offered()
    {
        var client = new OpenAiPolicyChatClient(new ThrowingChatClient(() => new InvalidOperationException()));

        Should.Throw<NotSupportedException>(() => client.GetStreamingResponseAsync("hi"));
    }

    private sealed class ThrowingChatClient : IChatClient
    {
        private readonly Func<Exception> _exception;

        public ThrowingChatClient(Func<Exception> exception)
        {
            _exception = exception;
        }

        public Task<ChatResponse> GetResponseAsync(System.Collections.Generic.IEnumerable<ChatMessage> messages,
            ChatOptions? options = null, CancellationToken cancellationToken = default) => throw _exception();

        public System.Collections.Generic.IAsyncEnumerable<ChatResponseUpdate> GetStreamingResponseAsync(
            System.Collections.Generic.IEnumerable<ChatMessage> messages, ChatOptions? options = null,
            CancellationToken cancellationToken = default) => throw new InvalidOperationException();

        public object? GetService(Type serviceType, object? serviceKey = null) => null;

        public void Dispose()
        {
        }
    }
}
