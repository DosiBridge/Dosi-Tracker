using Dosi.Tracker.AiAssist.Pipeline;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.AiAssist;

// The per-member AI lock is process-wide in tests, and every AI test signs in as the same member: AI test classes run
// one after another in the shared collection.
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreAiInsightRunnerTests : AiInsightRunnerTests<TrackerEntityFrameworkCoreTestModule>
{
}
