using System.Threading;
using System.Threading.Tasks;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Domain service for workspace-scoped activity and time summary age retention (REQ-DAT-01), run by
/// <see cref="ActivityRetentionWorker"/> for the workspaces whose owner set an activity retention policy.
/// </summary>
public interface IActivityRetentionService
{
    Task<ActivityRetentionResult> CleanExpiredActivitiesAsync(
        ActivityRetentionPolicy policy,
        CancellationToken cancellationToken = default);
}
