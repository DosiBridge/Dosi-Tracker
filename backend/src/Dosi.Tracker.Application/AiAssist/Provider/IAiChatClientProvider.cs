using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>
/// The only way AI assist reaches a model. The application registers <see cref="NullAiChatClientProvider"/> (no AI),
/// the API host replaces it with its OpenAI provider, and tests with a scripted fake, so no test can reach a provider.
/// Provider SDK types never leave the host: features see <see cref="IChatClient"/> and <see cref="AiProviderException"/>.
/// </summary>
public interface IAiChatClientProvider
{
    AiProviderState State { get; }

    /// <summary>Null unless <see cref="State"/> is <see cref="AiProviderState.Configured"/>.</summary>
    AiProviderDescriptor? Descriptor { get; }

    /// <summary>The client a feature calls, with its model (<c>AI:Features:{ConfigKey}:Model</c>, else
    /// <c>AI:OpenAI:Model</c>); null unless <see cref="State"/> is <see cref="AiProviderState.Configured"/>. Provider
    /// failures surface as <see cref="AiProviderException"/>.</summary>
    /// <param name="featureId">One of <see cref="AiFeatureIds"/>.</param>
    IChatClient? GetClient(string featureId);
}
