using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;

namespace Dosi.Tracker.AiAssist.Fakes;

/// <summary>
/// A configured provider whose every feature talks to one scripted <see cref="FakeChatClient"/>. Swap it in with
/// <c>services.Replace(ServiceDescriptor.Singleton&lt;IAiChatClientProvider&gt;(provider))</c> (AfterAddApplication, or
/// <c>ConfigureTestServerServices</c> for HTTP tests). Set <see cref="State"/> to test an unconfigured server.
/// </summary>
public sealed class FakeAiChatClientProvider : IAiChatClientProvider
{
    private readonly List<string> _requestedFeatures = new();

    public FakeAiChatClientProvider(FakeChatClient? client = null)
    {
        Client = client ?? new FakeChatClient();
    }

    public FakeChatClient Client { get; }

    public AiProviderState State { get; set; } = AiProviderState.Configured;

    public AiProviderDescriptor? Descriptor => State == AiProviderState.Configured
        ? new AiProviderDescriptor("Fake", "Fake", FakeChatClient.DefaultModel, SupportsJsonSchema: true, SupportsTools: true)
        : null;

    /// <summary>The feature ids clients were asked for, in order.</summary>
    public IReadOnlyList<string> RequestedFeatures
    {
        get
        {
            lock (_requestedFeatures)
            {
                return _requestedFeatures.ToList();
            }
        }
    }

    public IChatClient? GetClient(string featureId)
    {
        if (!AiFeatureIds.IsKnown(featureId))
        {
            throw new ArgumentException($"Unknown AI feature '{featureId}'.", nameof(featureId));
        }

        lock (_requestedFeatures)
        {
            _requestedFeatures.Add(featureId);
        }

        return State == AiProviderState.Configured ? Client : null;
    }
}
