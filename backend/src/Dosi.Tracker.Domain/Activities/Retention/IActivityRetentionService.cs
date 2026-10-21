using System.Threading;
using System.Threading.Tasks;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Domain service for workspace-scoped activity and time summary age retention (REQ-DAT-01).
/// Dormant unless explicitly invoked.
/// </summary>
public interface IActivityRetentionService
{
    Task<ActivityRetentionResult> CleanExpiredActivitiesAsync(
        ActivityRetentionPolicy policy,
        CancellationToken cancellationToken = default);
}
