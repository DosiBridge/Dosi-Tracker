using System.Threading;
using System.Threading.Tasks;

namespace Dosi.Tracker.Activities.Retention;

/// <summary>
/// Domain service responsible for workspace-scoped age-based capture (screenshot/thumb/webcam) cleanup.
/// </summary>
public interface ICaptureRetentionService
{
    /// <summary>
    /// Cleans expired captures for a specified workspace according to policy.
    /// Operates in bounded batches, preserves records at or newer than cutoff and unrelated workspaces,
    /// verifies canonical blob keys, safely handles blob deletion failures, and converges on retry.
    /// </summary>
    Task<CaptureRetentionResult> CleanExpiredCapturesAsync(
        CaptureRetentionPolicy policy,
        CancellationToken cancellationToken = default);
}
