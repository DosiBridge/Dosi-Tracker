using Dosi.Tracker.AiAssist.Provider;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>The application's default provider: AI is off, and no client exists for any feature.</summary>
public class NullAiChatClientProviderTests
{
    [Fact]
    public void Ai_Is_Off_For_Every_Feature()
    {
        var provider = new NullAiChatClientProvider();

        provider.State.ShouldBe(AiProviderState.Disabled);
        provider.Descriptor.ShouldBeNull();
        foreach (var feature in AiFeatureIds.All)
        {
            provider.GetClient(feature.Id).ShouldBeNull();
        }
    }
}
