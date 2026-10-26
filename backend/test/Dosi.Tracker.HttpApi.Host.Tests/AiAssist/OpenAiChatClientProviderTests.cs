using System;
using System.Collections.Generic;
using System.ClientModel.Primitives;
using System.Linq;
using System.Net.Http;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging;
using OpenAI;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The host's OpenAI provider against a scripted in-memory transport: what reaches the wire (strict json_schema,
/// max_completion_tokens, reasoning effort, organization and project headers, the configured endpoint), the retries of
/// 408/429/5xx (including OpenAI's intermittent 520) and network errors, and failures that leave as
/// AiProviderException without any provider text. The fake key is never logged.
/// </summary>
public class OpenAiChatClientProviderTests
{
    private const string FakeKey = "test-not-a-key";

    private readonly ScriptedOpenAiTransport _transport = new();
    private readonly ListLogger _logger = new();
    private readonly List<TimeSpan> _waits = new();
    private OpenAIClientOptions? _clientOptions;

    private static AiOptions Options(Action<AiOptions>? change = null)
    {
        var options = new AiOptions { Enabled = true, OpenAI = { Model = "gpt-5.4-mini" } };
        change?.Invoke(options);
        return options;
    }

    private OpenAiChatClientProvider Create(AiOptions options, string? apiKey = FakeKey, bool isDevelopment = false)
    {
        return new OpenAiChatClientProvider(options, apiKey, isDevelopment, _logger, clientOptions =>
        {
            _clientOptions = clientOptions;
            // The provider's own policy, without the real back-off waits.
            var policy = clientOptions.RetryPolicy.ShouldBeOfType<OpenAiRetryPolicy>();
            clientOptions.RetryPolicy = new NoWaitRetryPolicy(policy.MaxRetries, _waits);
            clientOptions.Transport = new HttpClientPipelineTransport(new HttpClient(_transport));
        });
    }

    private static ChatOptions SchemaOptions() => new()
    {
        MaxOutputTokens = 600,
        Reasoning = new ReasoningOptions { Effort = ReasoningEffort.Low },
        ResponseFormat = ChatResponseFormat.ForJsonSchema(
            JsonDocument.Parse("""{"type":"object","properties":{"headline":{"type":"string"}},"required":["headline"],"additionalProperties":false}""").RootElement,
            "period_summary", "A summary card.")
    };

    private static Task<ChatResponse> AskAsync(IChatClient client, ChatOptions? options = null, CancellationToken cancellationToken = default) =>
        client.GetResponseAsync(new[] { new ChatMessage(ChatRole.User, "Summarize.") }, options, cancellationToken);

    private void ShouldNotLeak(Exception exception)
    {
        exception.InnerException.ShouldBeNull();
        exception.Message.ShouldNotContain("PROVIDER-DETAIL");
        exception.ToString().ShouldNotContain("PROVIDER-DETAIL");
        _logger.Entries.ShouldAllBe(e => !e.Message.Contains(FakeKey));
    }

    [Fact]
    public void Off_By_Default_Builds_No_Client_And_Says_So()
    {
        var provider = Create(new AiOptions());

        provider.State.ShouldBe(AiProviderState.Disabled);
        provider.Descriptor.ShouldBeNull();
        provider.GetClient(AiFeatureIds.PeriodSummaryId).ShouldBeNull();
        _clientOptions.ShouldBeNull();
        _logger.Entries.ShouldHaveSingleItem().Level.ShouldBe(LogLevel.Information);
        _logger.Entries[0].Message.ShouldContain("Disabled");
    }

    [Fact]
    public void Turned_On_But_Incomplete_Warns_Without_The_Key()
    {
        var missingKey = Create(Options(), apiKey: "  ");
        missingKey.State.ShouldBe(AiProviderState.MissingApiKey);
        missingKey.GetClient(AiFeatureIds.AskId).ShouldBeNull();

        var badEndpoint = Create(Options(o => o.OpenAI.BaseUrl = "https://evil.example.com/v1"));
        badEndpoint.State.ShouldBe(AiProviderState.InvalidEndpoint);

        _logger.Entries.Count.ShouldBe(2);
        _logger.Entries.ShouldAllBe(e => e.Level == LogLevel.Warning);
        _logger.Entries[0].Message.ShouldContain("MissingApiKey");
        _logger.Entries[1].Message.ShouldContain("InvalidEndpoint");
        _logger.Entries.ShouldAllBe(e => !e.Message.Contains(FakeKey) && !e.Message.Contains("evil.example.com"));
    }

    [Fact]
    public void Configured_Logs_One_Line_With_Provider_And_Model_Only()
    {
        var provider = Create(Options(o => o.OpenAI.OrganizationId = "org-test"));

        provider.State.ShouldBe(AiProviderState.Configured);
        provider.Descriptor.ShouldBe(new AiProviderDescriptor("OpenAI", "OpenAI", "gpt-5.4-mini", true, true));
        var line = _logger.Entries.ShouldHaveSingleItem();
        line.Level.ShouldBe(LogLevel.Information);
        line.Message.ShouldContain("Configured");
        line.Message.ShouldContain("gpt-5.4-mini");
        line.Message.ShouldNotContain(FakeKey);
    }

    [Fact]
    public void The_Sdk_Gets_The_Configured_Client_Options()
    {
        Create(Options(o =>
        {
            o.OpenAI.OrganizationId = " org-test ";
            o.OpenAI.ProjectId = "";
            o.RequestTimeoutSeconds = 20;
            o.MaxRetries = 3;
        }));

        var clientOptions = _clientOptions.ShouldNotBeNull();
        clientOptions.Endpoint.ShouldBeNull();
        clientOptions.OrganizationId.ShouldBe("org-test");
        clientOptions.ProjectId.ShouldBeNull();
        clientOptions.UserAgentApplicationId.ShouldBe("dosi-tracker");
        clientOptions.NetworkTimeout.ShouldBe(TimeSpan.FromSeconds(20));
        clientOptions.RetryPolicy.ShouldBeOfType<NoWaitRetryPolicy>().MaxRetries.ShouldBe(3);
    }

    [Fact]
    public void One_Client_Per_Model_And_Features_May_Use_Their_Own()
    {
        var provider = Create(Options(o => o.Features["taskdrafts"] = new AiFeatureOptions { Model = "gpt-4.1-mini" }));

        var summary = provider.GetClient(AiFeatureIds.PeriodSummaryId).ShouldBeOfType<OpenAiPolicyChatClient>();
        provider.GetClient(AiFeatureIds.MemberDaySummaryId).ShouldBeSameAs(summary);
        var drafts = provider.GetClient(AiFeatureIds.TaskDraftsId);
        drafts.ShouldNotBeSameAs(summary);
        provider.GetClient(AiFeatureIds.TaskDraftsId).ShouldBeSameAs(drafts);

        Should.Throw<ArgumentException>(() => provider.GetClient("unknown-feature"));
    }

    [Fact]
    public async Task A_Schema_Request_Is_Sent_As_Strict_Structured_Output()
    {
        _transport.Completion("{\"headline\":\"{M1} tracked {F1}\"}");
        var provider = Create(Options(o => { o.OpenAI.OrganizationId = "org-test"; o.OpenAI.ProjectId = "proj-test"; }));
        var options = SchemaOptions();

        var response = await AskAsync(provider.GetClient(AiFeatureIds.PeriodSummaryId)!, options);

        response.Text.ShouldBe("{\"headline\":\"{M1} tracked {F1}\"}");
        response.FinishReason.ShouldBe(ChatFinishReason.Stop);
        response.Usage!.InputTokenCount.ShouldBe(120);
        response.Usage.OutputTokenCount.ShouldBe(30);
        response.Usage.CachedInputTokenCount.ShouldBe(20);
        response.Usage.ReasoningTokenCount.ShouldBe(10);
        options.AdditionalProperties.ShouldBeNull();

        var request = _transport.Requests.ShouldHaveSingleItem();
        request.Uri.ShouldBe(new Uri("https://api.openai.com/v1/chat/completions"));
        request.Headers["Authorization"].ShouldBe("Bearer " + FakeKey);
        request.Headers["OpenAI-Organization"].ShouldBe("org-test");
        request.Headers["OpenAI-Project"].ShouldBe("proj-test");
        request.Headers["User-Agent"].ShouldContain("dosi-tracker");

        using var body = JsonDocument.Parse(request.Body);
        var root = body.RootElement;
        root.GetProperty("model").GetString().ShouldBe("gpt-5.4-mini");
        root.GetProperty("max_completion_tokens").GetInt32().ShouldBe(600);
        root.GetProperty("reasoning_effort").GetString().ShouldBe("low");
        root.TryGetProperty("max_tokens", out _).ShouldBeFalse();
        root.TryGetProperty("temperature", out _).ShouldBeFalse();
        var format = root.GetProperty("response_format");
        format.GetProperty("type").GetString().ShouldBe("json_schema");
        format.GetProperty("json_schema").GetProperty("name").GetString().ShouldBe("period_summary");
        format.GetProperty("json_schema").GetProperty("strict").GetBoolean().ShouldBeTrue();
    }

    [Fact]
    public async Task A_Configured_Endpoint_Receives_The_Calls()
    {
        _transport.Completion("plain answer");
        var provider = Create(Options(o =>
        {
            o.Provider = AiOptions.ProviderAzureOpenAi;
            o.OpenAI.Model = "tracker-deployment";
            o.OpenAI.BaseUrl = "https://example-resource.openai.azure.com/openai/v1/";
            o.OpenAI.AllowedHosts = "example-resource.openai.azure.com";
        }));

        (await AskAsync(provider.GetClient(AiFeatureIds.AskId)!)).Text.ShouldBe("plain answer");

        var request = _transport.Requests.ShouldHaveSingleItem();
        request.Uri.ToString().ShouldBe("https://example-resource.openai.azure.com/openai/v1/chat/completions");
        JsonDocument.Parse(request.Body).RootElement.GetProperty("model").GetString().ShouldBe("tracker-deployment");
    }

    [Theory]
    [InlineData(520)]
    [InlineData(500)]
    [InlineData(503)]
    [InlineData(529)]
    [InlineData(408)]
    [InlineData(429)]
    public async Task Transient_Statuses_Are_Retried_With_Backoff(int status)
    {
        _transport.Status(status).Status(status).Completion();
        var provider = Create(Options());

        var response = await AskAsync(provider.GetClient(AiFeatureIds.PeriodSummaryId)!, SchemaOptions());

        response.Text.ShouldBe("{\"ok\":true}");
        _transport.Requests.Count.ShouldBe(3);
        _waits.Count.ShouldBe(2);
        _waits.ShouldAllBe(w => w > TimeSpan.Zero);
    }

    [Fact]
    public async Task Exhausted_Retries_Leave_As_A_Server_Error_Without_Provider_Text()
    {
        _transport.Status(520).Status(520).Status(520);
        var provider = Create(Options());

        var exception = await Should.ThrowAsync<AiProviderException>(() => AskAsync(provider.GetClient(AiFeatureIds.PeriodSummaryId)!));

        exception.Kind.ShouldBe(AiProviderErrorKind.ServerError);
        exception.Status.ShouldBe(520);
        exception.IsTransient.ShouldBeTrue();
        _transport.Requests.Count.ShouldBe(3);
        ShouldNotLeak(exception);
    }

    [Theory]
    [InlineData(401, AiProviderErrorKind.Auth)]
    [InlineData(403, AiProviderErrorKind.Auth)]
    [InlineData(400, AiProviderErrorKind.BadRequest)]
    [InlineData(404, AiProviderErrorKind.BadRequest)]
    public async Task Permanent_Failures_Are_Not_Retried(int status, AiProviderErrorKind kind)
    {
        _transport.Status(status);
        var provider = Create(Options());

        var exception = await Should.ThrowAsync<AiProviderException>(() => AskAsync(provider.GetClient(AiFeatureIds.AskId)!));

        exception.Kind.ShouldBe(kind);
        exception.Status.ShouldBe(status);
        _transport.Requests.Count.ShouldBe(1);
        ShouldNotLeak(exception);
    }

    [Fact]
    public async Task A_Short_Retry_After_Is_Waited_For()
    {
        _transport.Status(429, retryAfter: "2").Completion();
        var provider = Create(Options());

        await AskAsync(provider.GetClient(AiFeatureIds.AskId)!);

        _transport.Requests.Count.ShouldBe(2);
        _waits.ShouldHaveSingleItem().ShouldBe(TimeSpan.FromSeconds(2));
    }

    [Fact]
    public async Task A_Long_Retry_After_Is_Reported_Instead_Of_Waited_For()
    {
        _transport.Status(429, retryAfter: "30");
        var provider = Create(Options());

        var exception = await Should.ThrowAsync<AiProviderException>(() => AskAsync(provider.GetClient(AiFeatureIds.AskId)!));

        exception.Kind.ShouldBe(AiProviderErrorKind.RateLimited);
        exception.RetryAfterSeconds.ShouldBe(30);
        _transport.Requests.Count.ShouldBe(1);
        _waits.ShouldBeEmpty();
        ShouldNotLeak(exception);
    }

    [Fact]
    public async Task Network_Failures_Are_Retried_Then_Reported()
    {
        _transport.NetworkFailure().NetworkFailure().NetworkFailure();
        var provider = Create(Options());

        var exception = await Should.ThrowAsync<AiProviderException>(() => AskAsync(provider.GetClient(AiFeatureIds.AskId)!));

        exception.Kind.ShouldBe(AiProviderErrorKind.Network);
        exception.Status.ShouldBeNull();
        _transport.Requests.Count.ShouldBe(3);
        ShouldNotLeak(exception);
    }

    [Fact]
    public async Task An_Attempt_Past_The_Timeout_Is_A_Timeout()
    {
        _transport.Hang();
        var provider = Create(Options(o => { o.RequestTimeoutSeconds = 1; o.MaxRetries = 0; }));

        var exception = await Should.ThrowAsync<AiProviderException>(() => AskAsync(provider.GetClient(AiFeatureIds.AskId)!));

        exception.Kind.ShouldBe(AiProviderErrorKind.Timeout);
        _transport.Requests.Count.ShouldBe(1);
        ShouldNotLeak(exception);
    }

    [Fact]
    public async Task The_Callers_Cancellation_Propagates_Unchanged()
    {
        _transport.Hang();
        var provider = Create(Options());
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(100));

        var exception = await Should.ThrowAsync<OperationCanceledException>(
            () => AskAsync(provider.GetClient(AiFeatureIds.AskId)!, cancellationToken: cancellation.Token));

        exception.ShouldNotBeOfType<AiProviderException>();
        _transport.Requests.Count.ShouldBe(1);
    }

    [Fact]
    public async Task An_Unscripted_Request_Never_Leaves_The_Test()
    {
        var provider = Create(Options(o => o.MaxRetries = 0));

        // The transport's refusal is a programming error, not a provider failure: it propagates as it is.
        (await Should.ThrowAsync<InvalidOperationException>(() => AskAsync(provider.GetClient(AiFeatureIds.AskId)!)))
            .Message.ShouldStartWith("Unscripted provider request");
        _transport.Requests.ShouldHaveSingleItem();
    }

    /// <summary>The provider's retry policy, recording its back-off instead of waiting it out.</summary>
    private sealed class NoWaitRetryPolicy : OpenAiRetryPolicy
    {
        private readonly List<TimeSpan> _waits;

        public NoWaitRetryPolicy(int maxRetries, List<TimeSpan> waits)
            : base(maxRetries)
        {
            _waits = waits;
        }

        protected override void Wait(TimeSpan time, CancellationToken cancellationToken) => _waits.Add(time);

        protected override Task WaitAsync(TimeSpan time, CancellationToken cancellationToken)
        {
            _waits.Add(time);
            return Task.CompletedTask;
        }
    }
}
