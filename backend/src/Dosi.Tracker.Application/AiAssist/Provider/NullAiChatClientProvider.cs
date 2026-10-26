using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>No AI provider: every process but the configured API host (the DB migrator, test hosts) runs with AI off.</summary>
public sealed class NullAiChatClientProvider : IAiChatClientProvider
{
    public AiProviderState State => AiProviderState.Disabled;

    public AiProviderDescriptor? Descriptor => null;

    public IChatClient? GetClient(string featureId) => null;
}
