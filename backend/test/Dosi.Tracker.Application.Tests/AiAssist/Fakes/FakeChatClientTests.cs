using System;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Fakes;

/// <summary>The test model itself: scripted answers in order, recorded calls, and a loud failure for anything
/// unscripted.</summary>
public class FakeChatClientTests
{
    private static Task<ChatResponse> AskAsync(IChatClient client, ChatOptions? options = null, CancellationToken token = default) =>
        client.GetResponseAsync(new[] { new ChatMessage(ChatRole.System, "policy"), new ChatMessage(ChatRole.User, "question") },
            options, token);

    [Fact]
    public async Task Answers_Come_In_Script_Order_And_Calls_Are_Recorded()
    {
        var client = new FakeChatClient()
            .EnqueueJson("""{"headline":"{M1} tracked {F1}"}""")
            .EnqueueText("not json")
            .EnqueueFinishReason(ChatFinishReason.Length, "{\"head");

        var options = new ChatOptions { MaxOutputTokens = 600 };
        var first = await AskAsync(client, options);
        options.MaxOutputTokens = 1; // the recorded options are a copy
        var second = await AskAsync(client);
        var third = await AskAsync(client);

        first.Text.ShouldBe("""{"headline":"{M1} tracked {F1}"}""");
        first.FinishReason.ShouldBe(ChatFinishReason.Stop);
        first.ModelId.ShouldBe(FakeChatClient.DefaultModel);
        first.Usage!.InputTokenCount.ShouldBe(400);
        second.Text.ShouldBe("not json");
        third.FinishReason.ShouldBe(ChatFinishReason.Length);

        client.Calls.Count.ShouldBe(3);
        client.Calls[0].Options!.MaxOutputTokens.ShouldBe(600);
        client.Calls[0].Messages.Count.ShouldBe(2);
        client.Calls[0].AllText.ShouldContain("question");
        client.Pending.ShouldBe(0);
    }

    [Fact]
    public async Task An_Unscripted_Call_Fails_Loudly()
    {
        var client = new FakeChatClient();

        (await Should.ThrowAsync<InvalidOperationException>(() => AskAsync(client))).Message.ShouldContain("unscripted");
        client.Calls.Count.ShouldBe(1);
    }

    [Fact]
    public void Json_Scripts_Must_Be_Json()
    {
        Should.Throw<JsonException>(() => new FakeChatClient().EnqueueJson("{oops"));
    }

    [Fact]
    public async Task Tool_Calls_Throws_And_Usage_Are_Scripted()
    {
        var usage = new UsageDetails { InputTokenCount = 7, OutputTokenCount = 3 };
        var client = new FakeChatClient()
            .EnqueueToolCall("get_time_summary", """{"from":"2026-10-01","to":"2026-10-07"}""", usage)
            .EnqueueThrow(new AiProviderException(AiProviderErrorKind.RateLimited, 429, 12));

        var call = await AskAsync(client);

        call.FinishReason.ShouldBe(ChatFinishReason.ToolCalls);
        call.Usage.ShouldBeSameAs(usage);
        var function = call.Messages.Single().Contents.OfType<FunctionCallContent>().Single();
        function.Name.ShouldBe("get_time_summary");
        function.CallId.ShouldNotBeNullOrEmpty();
        function.Arguments!["from"]!.ToString().ShouldBe("2026-10-01");

        (await Should.ThrowAsync<AiProviderException>(() => AskAsync(client))).RetryAfterSeconds.ShouldBe(12);
    }

    [Fact]
    public async Task A_Delay_Holds_The_Next_Answer_And_Honours_Cancellation()
    {
        var client = new FakeChatClient()
            .EnqueueDelay(TimeSpan.FromMilliseconds(20)).EnqueueText("late")
            .EnqueueDelay(TimeSpan.FromMinutes(5)).EnqueueText("never");

        (await AskAsync(client)).Text.ShouldBe("late");

        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(50));
        await Should.ThrowAsync<OperationCanceledException>(() => AskAsync(client, token: cancellation.Token));
        client.Calls.Count.ShouldBe(2);
    }

    [Fact]
    public void The_Fake_Provider_Is_Configured_And_Hands_Out_The_Shared_Client()
    {
        var provider = new FakeAiChatClientProvider();

        provider.State.ShouldBe(AiProviderState.Configured);
        provider.Descriptor!.DisplayName.ShouldBe("Fake");
        provider.Descriptor.DefaultModel.ShouldBe("fake-model");
        provider.GetClient(AiFeatureIds.AskId).ShouldBeSameAs(provider.Client);
        provider.GetClient(AiFeatureIds.TaskDraftsId).ShouldBeSameAs(provider.Client);
        provider.RequestedFeatures.ShouldBe(new[] { "ask", "task-drafts" });
        Should.Throw<ArgumentException>(() => provider.GetClient("nope"));

        provider.State = AiProviderState.MissingApiKey;
        provider.Descriptor.ShouldBeNull();
        provider.GetClient(AiFeatureIds.AskId).ShouldBeNull();
    }

    [Fact]
    public void The_Fake_Describes_Itself_And_Does_Not_Stream()
    {
        var client = new FakeChatClient();

        client.GetService(typeof(ChatClientMetadata)).ShouldBeOfType<ChatClientMetadata>().DefaultModelId.ShouldBe("fake-model");
        client.GetService(typeof(FakeChatClient)).ShouldBeSameAs(client);
        client.GetService(typeof(string)).ShouldBeNull();
        client.GetService(typeof(FakeChatClient), "key").ShouldBeNull();
        Should.Throw<NotSupportedException>(() => client.GetStreamingResponseAsync("x"));
    }
}
