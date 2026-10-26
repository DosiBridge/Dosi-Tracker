using System;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Volo.Abp.DependencyInjection;
using Volo.Abp.DistributedLocking;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// One AI request in flight per member and workspace: a second one while the first still waits on the provider is
/// refused at once (<c>Tracker:AiRateLimited</c> busy), which also bounds how far a member can overshoot an allowance.
/// The lock is ABP's distributed lock, local to the instance unless a distributed provider is configured.
/// </summary>
public class AiUserLock : ITransientDependency
{
    /// <summary>The retry-after of a busy refusal.</summary>
    public const int BusyRetryAfterSeconds = 3;

    private readonly IAbpDistributedLock _distributedLock;

    public AiUserLock(IAbpDistributedLock distributedLock)
    {
        _distributedLock = distributedLock;
    }

    /// <summary>"Tracker:Ai:User:{workspace:N}:{user:N}".</summary>
    public static string KeyFor(Guid workspaceId, Guid userId) => $"Tracker:Ai:User:{workspaceId:N}:{userId:N}";

    /// <summary>Takes the caller's lock without waiting; dispose the handle to release it.</summary>
    public virtual async Task<IAbpDistributedLockHandle> AcquireAsync(
        AiCallerContext caller, string featureId, CancellationToken cancellationToken = default)
    {
        var handle = await _distributedLock.TryAcquireAsync(
            KeyFor(caller.WorkspaceId, caller.UserId), TimeSpan.Zero, cancellationToken);

        return handle ?? throw AiErrors.RateLimited(AiErrors.BusyReason, BusyRetryAfterSeconds, featureId);
    }
}
