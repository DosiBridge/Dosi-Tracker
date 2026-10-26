using System;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Erasure;

/// <summary>
/// A deleted account whose tracked data the erasure sweep still has to scrub (<see cref="AccountEraser.ScrubAsync"/>):
/// its captures are deleted and its time blocks lose window titles, application names, notes and device names. Done
/// once <see cref="CompletedTime"/> is set.
/// </summary>
public class AccountErasure : CreationAuditedAggregateRoot<Guid>
{
    public Guid UserId { get; private set; }

    public DateTime? CompletedTime { get; private set; }

    protected AccountErasure()
    {
    }

    public AccountErasure(Guid id, Guid userId)
        : base(id)
    {
        UserId = userId;
    }

    public void MarkCompleted(DateTime completedTime)
    {
        CompletedTime ??= completedTime;
    }
}
