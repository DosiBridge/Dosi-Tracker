using Dosi.Tracker.AiAssist.Assistant;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

/// <summary>Ask AI (AI BE-11) on EF Core / SQLite.</summary>
public static class EfCoreAiAssistantTests
{
    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreAiToolScopingTests : AiToolScopingTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreAiAssistantInjectionTests : AiAssistantInjectionTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreAiAssistantLoopTests : AiAssistantLoopTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreAiAssistantValidationTests : AiAssistantValidationTests<TrackerEntityFrameworkCoreTestModule>
    {
    }

    [Collection(TrackerTestConsts.CollectionDefinitionName)]
    public class EfCoreAiConversationStoreTests : AiConversationStoreTests<TrackerEntityFrameworkCoreTestModule>
    {
    }
}
