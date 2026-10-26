using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Sums the usage of every provider call of one Ask AI turn. Placed <em>inside</em> function invocation
/// (<see cref="AiAssistantRunner"/> builds <c>UseFunctionInvocation(...).Use(inner =&gt; new UsageMeteringChatClient(inner,
/// turn.Usage))</c>, and <see cref="Microsoft.Extensions.AI.ChatClientBuilder"/> makes the first <c>Use</c> the outermost
/// layer): function invocation calls the provider once per round, and each of those calls passes through here, so the
/// turn's usage is the true total across every round, not just the last one.
/// </summary>
public sealed class UsageMeteringChatClient : DelegatingChatClient
{
    private readonly AiTurnUsage _usage;

    public UsageMeteringChatClient(IChatClient innerClient, AiTurnUsage usage)
        : base(innerClient)
    {
        _usage = usage;
    }

    public override async Task<ChatResponse> GetResponseAsync(
        IEnumerable<ChatMessage> messages, ChatOptions? options = null, CancellationToken cancellationToken = default)
    {
        var response = await base.GetResponseAsync(messages, options, cancellationToken);
        _usage.Add(response.Usage);
        return response;
    }
}
