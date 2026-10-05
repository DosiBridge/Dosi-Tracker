using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Projects;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// The one definition of an occupied seat, shared by the plan's seat limit (team service), the monthly invoice and the
/// platform console: a distinct user with an active account in the workspace who holds a tracked membership (any role
/// but <see cref="TrackerRoles.ClientProjectRole"/>) on a project that still exists. Clients follow projects without
/// tracking and take no seat; deleting or deactivating an account (removal from the workspace) frees its seat even
/// where a membership row was left behind; and a membership whose user has no account in the workspace (a stray id)
/// never occupies, or bills, a seat.
/// </summary>
public class SeatCounter : DomainService
{
    /// <summary>How long a change that may take a seat waits for another one of the same workspace to finish.</summary>
    public static readonly TimeSpan SeatLockTimeout = TimeSpan.FromSeconds(30);

    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<IdentityUser, Guid> _userRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IAbpDistributedLock _distributedLock;

    public SeatCounter(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<IdentityUser, Guid> userRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IDataFilter dataFilter,
        IAbpDistributedLock distributedLock)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _userRepository = userRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _dataFilter = dataFilter;
        _distributedLock = distributedLock;
    }

    /// <summary>The name of the lock that serializes the seat-taking changes of a workspace (null: the host).</summary>
    public static string SeatLockName(Guid? tenantId) => $"Tracker:Seats:{tenantId?.ToString("N") ?? "host"}";

    /// <summary>
    /// Serializes the changes that can take a seat in the current workspace — adding or inviting a member, turning a
    /// client into a tracked member — so that two of them cannot both see the last free seat. The caller checks
    /// <see cref="CheckSeatAvailableAsync"/> and saves the membership while holding the lock, and commits its unit of
    /// work before releasing it, so the next change counts that seat. Held across server instances when a distributed
    /// lock provider is configured. Throws when another change keeps it longer than <see cref="SeatLockTimeout"/>.
    /// </summary>
    public virtual async Task<IAbpDistributedLockHandle> LockAsync()
    {
        var handle = await _distributedLock.TryAcquireAsync(SeatLockName(CurrentTenant.Id), SeatLockTimeout);
        return handle ?? throw new AbpException(
            "Another change of this workspace's team is still being saved; try again in a moment.");
    }

    /// <summary>Occupied seats in the current workspace.</summary>
    public virtual async Task<int> CountAsync()
    {
        var seatMemberships = await GetSeatMembershipsAsync();
        return await AsyncExecuter.CountAsync(seatMemberships.Select(m => m.UserId).Distinct());
    }

    /// <summary>True when the user already occupies a seat in the current workspace (joining another project is free).</summary>
    public virtual async Task<bool> HoldsSeatAsync(Guid userId)
    {
        var seatMemberships = await GetSeatMembershipsAsync();
        return await AsyncExecuter.AnyAsync(seatMemberships.Where(m => m.UserId == userId));
    }

    /// <summary>Occupied seats of every workspace (host aggregation), keyed by tenant id. Workspaces without
    /// an occupied seat are absent; memberships on the host side (no tenant) are ignored.</summary>
    public virtual async Task<Dictionary<Guid, int>> CountByTenantAsync()
    {
        using (_dataFilter.Disable<IMultiTenant>())
        {
            var seatMemberships = await GetSeatMembershipsAsync();
            var seats = await AsyncExecuter.ToListAsync(seatMemberships
                .Where(m => m.TenantId != null)
                .Select(m => new { m.TenantId, m.UserId })
                .Distinct());

            return seats
                .GroupBy(s => s.TenantId!.Value)
                .ToDictionary(g => g.Key, g => g.Count());
        }
    }

    /// <summary>Throws <c>Tracker:SeatLimitReached</c> when giving <paramref name="userId"/> (null: a person who has no
    /// account yet) a tracked membership would take a new seat beyond the current workspace's plan limit. A user who
    /// already holds a seat can always join more projects; a workspace without a subscription (or on an unknown or
    /// unlimited plan) has no limit. A Client membership needs no seat, so callers skip this check for it.</summary>
    public virtual async Task CheckSeatAvailableAsync(Guid? userId)
    {
        var plan = await FindCurrentPlanAsync();
        if (plan == null || plan.MaxSeats <= 0)
        {
            return;
        }

        if (userId.HasValue && await HoldsSeatAsync(userId.Value))
        {
            return;
        }

        if (await CountAsync() >= plan.MaxSeats)
        {
            throw new BusinessException(TrackerDomainErrorCodes.SeatLimitReached)
                .WithData("maxSeats", plan.MaxSeats)
                .WithData("plan", plan.Name);
        }
    }

    /// <summary>Throws <c>Tracker:SeatLimitReached</c> when the current workspace occupies more seats than
    /// <paramref name="plan"/> allows (a plan change must not leave the workspace over its limit).</summary>
    public virtual async Task CheckFitsPlanAsync(Plan plan)
    {
        if (plan.MaxSeats <= 0)
        {
            return;
        }

        var seatsUsed = await CountAsync();
        if (seatsUsed > plan.MaxSeats)
        {
            throw new BusinessException(TrackerDomainErrorCodes.SeatLimitReached)
                .WithData("maxSeats", plan.MaxSeats)
                .WithData("plan", plan.Name)
                .WithData("seatsUsed", seatsUsed);
        }
    }

    private async Task<Plan?> FindCurrentPlanAsync()
    {
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.TenantId == CurrentTenant.Id);
        return subscription == null ? null : await _planRepository.FindAsync(subscription.PlanId);
    }

    /// <summary>Memberships that occupy a seat: a tracked role (not Client, whatever its case or surrounding spaces) on
    /// a live project of the membership's workspace, held by an active account of that workspace. The deletion checks
    /// are explicit, so the result is the same whether or not the caller disabled the soft-delete filter.</summary>
    protected virtual async Task<IQueryable<ProjectMember>> GetSeatMembershipsAsync()
    {
        var members = await _memberRepository.GetQueryableAsync();
        var projects = await _projectRepository.GetQueryableAsync();
        var users = await _userRepository.GetQueryableAsync();
        var clientRole = TrackerRoles.ClientProjectRole.ToUpperInvariant();

        return members.Where(m =>
            m.Role.Trim().ToUpper() != clientRole &&
            projects.Any(p => p.Id == m.ProjectId && p.TenantId == m.TenantId && !p.IsDeleted) &&
            users.Any(u => u.Id == m.UserId && u.TenantId == m.TenantId && !u.IsDeleted && u.IsActive));
    }
}
