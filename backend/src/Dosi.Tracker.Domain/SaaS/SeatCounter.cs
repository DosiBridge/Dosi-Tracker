using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Identity;
using Microsoft.Extensions.Options;
using Dosi.Tracker.Projects;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;
using Volo.Abp.Identity;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.SaaS;

/// <summary>
/// The one definition of an occupied seat, shared by the plan's seat limit (team service), the monthly invoice and the
/// platform console: a distinct active member of the workspace (with an active account) who holds a tracked project
/// membership (any role but <see cref="TrackerRoles.ClientProjectRole"/>) on a project of that workspace that still exists.
/// </summary>
public class SeatCounter : TrackerDomainService
{
    /// <summary>How long a change that may take a seat waits for another one of the same workspace to finish.</summary>
    public static readonly TimeSpan SeatLockTimeout = TimeSpan.FromSeconds(30);

    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<IdentityUser, Guid> _userRepository;
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IDataFilter _dataFilter;
    private readonly IAbpDistributedLock _distributedLock;

    public SeatCounter(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<IdentityUser, Guid> userRepository,
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IDataFilter dataFilter,
        IAbpDistributedLock distributedLock)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _userRepository = userRepository;
        _workspaceMemberRepository = workspaceMemberRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _dataFilter = dataFilter;
        _distributedLock = distributedLock;
    }

    /// <summary>The name of the lock that serializes the seat-taking changes of a workspace.</summary>
    public static string SeatLockName(Guid? workspaceId) => $"Tracker:Seats:{workspaceId?.ToString("N") ?? "none"}";

    /// <summary>
    /// Serializes the changes that can take a seat in the current workspace — adding or inviting a member, turning a
    /// client into a tracked member — so that two of them cannot both see the last free seat. The caller checks
    /// <see cref="CheckSeatAvailableAsync"/> and saves the membership while holding the lock, and commits its unit of
    /// work before releasing it, so the next change counts that seat. Held across server instances when a distributed
    /// lock provider is configured. Throws when another change keeps it longer than <see cref="SeatLockTimeout"/>.
    /// </summary>
    public virtual async Task<IAbpDistributedLockHandle> LockAsync()
    {
        var handle = await _distributedLock.TryAcquireAsync(SeatLockName(CurrentWorkspace.Id), SeatLockTimeout);
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

    /// <summary>Occupied seats of every workspace (platform aggregation), keyed by workspace id. Workspaces without
    /// an occupied seat are absent.</summary>
    public virtual async Task<Dictionary<Guid, int>> CountByWorkspaceAsync()
    {
        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            var seatMemberships = await GetSeatMembershipsAsync();
            var seats = await AsyncExecuter.ToListAsync(seatMemberships
                .Select(m => new { m.WorkspaceId, m.UserId })
                .Distinct());

            return seats
                .GroupBy(s => s.WorkspaceId)
                .ToDictionary(g => g.Key, g => g.Count());
        }
    }

    /// <summary>Throws <c>Tracker:SeatLimitReached</c> when giving <paramref name="userId"/> (null: a person who has no
    /// account yet) a tracked membership would take a new seat beyond the current workspace's plan limit. A user who
    /// already holds a seat can always join more projects; a workspace without a subscription (or on an unknown or
    /// unlimited plan) has no limit, and neither has any workspace while billing is off (<see cref="BillingOptions"/>).
    /// A Client membership needs no seat, so callers skip this check for it.</summary>
    public virtual async Task CheckSeatAvailableAsync(Guid? userId)
    {
        if (!IsBillingEnabled)
        {
            return; // Without billing, plans set no seat limit.
        }

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
    /// <paramref name="plan"/> allows (a plan change must not leave the workspace over its limit). Never while billing is
    /// off.</summary>
    public virtual async Task CheckFitsPlanAsync(Plan plan)
    {
        if (!IsBillingEnabled || plan.MaxSeats <= 0)
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

    /// <summary>Whether this deployment bills workspaces at all (<see cref="BillingOptions"/>).</summary>
    protected bool IsBillingEnabled =>
        LazyServiceProvider.LazyGetRequiredService<IOptions<BillingOptions>>().Value.Enabled;

    private async Task<Plan?> FindCurrentPlanAsync()
    {
        var workspaceId = CurrentWorkspace.Id;
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == workspaceId);
        return subscription == null ? null : await _planRepository.FindAsync(subscription.PlanId);
    }

    /// <summary>Memberships that occupy a seat: a tracked role (not Client, whatever its case or surrounding spaces) on
    /// a live project of the membership's workspace, held by an active member of that workspace with an active account.
    /// The deletion checks are explicit, so the result is the same whether or not the caller disabled the soft-delete
    /// filter.</summary>
    protected virtual async Task<IQueryable<ProjectMember>> GetSeatMembershipsAsync()
    {
        var members = await _memberRepository.GetQueryableAsync();
        var projects = await _projectRepository.GetQueryableAsync();
        var users = await _userRepository.GetQueryableAsync();
        var workspaceMembers = await _workspaceMemberRepository.GetQueryableAsync();
        var clientRole = TrackerRoles.ClientProjectRole.ToUpperInvariant();

        return members.Where(m =>
            m.Role.Trim().ToUpper() != clientRole &&
            projects.Any(p => p.Id == m.ProjectId && p.WorkspaceId == m.WorkspaceId && !p.IsDeleted) &&
            workspaceMembers.Any(w => w.WorkspaceId == m.WorkspaceId && w.UserId == m.UserId && w.Status == WorkspaceMemberStatus.Active) &&
            users.Any(u => u.Id == m.UserId && !u.IsDeleted && u.IsActive));
    }
}
