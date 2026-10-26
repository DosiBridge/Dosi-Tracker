using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;

namespace Dosi.Tracker.AiAssist;

public static class AiHostServiceCollectionExtensions
{
    /// <summary>Replaces the application's "no AI" provider with the OpenAI provider. It still answers
    /// <see cref="AiProviderState.Disabled"/> until <c>AI:Enabled</c>, a key and a model are configured.</summary>
    public static IServiceCollection AddTrackerAiAssist(this IServiceCollection services)
    {
        services.Replace(ServiceDescriptor.Singleton<IAiChatClientProvider, OpenAiChatClientProvider>());
        return services;
    }
}
