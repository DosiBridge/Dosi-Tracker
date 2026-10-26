using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Linq;
using Volo.Abp.Users;

namespace Dosi.Tracker.AiAssist.Access;

/// <summary>Builds the <see cref="AiCallerContext"/> of the signed-in caller in the current workspace.</summary>
public class AiCallerContextFactory : ITransientDependency
{
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly ICurrentUser _currentUser;
    private readonly WorkspaceRoleLookup _roleLookup;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IAuthorizationService _authorizationService;
    private readonly IAsyncQueryableExecuter _asyncExecuter;
    private readonly WorkspaceSettingsManager _workspaceSettings;
    private readonly AiSettingsManager _aiSettings;
    private readonly SubscriptionManager _subscriptions;

    public AiCallerContextFactory(
        ICurrentWorkspace currentWorkspace,
        ICurrentUser currentUser,
        WorkspaceRoleLookup roleLookup,
        IRepository<ProjectMember, Guid> projectMemberRepository,
        IRepository<Plan, Guid> planRepository,
        IAuthorizationService authorizationService,
        IAsyncQueryableExecuter asyncExecuter,
        WorkspaceSettingsManager workspaceSettings,
        AiSettingsManager aiSettings,
        SubscriptionManager subscriptions)
    {
        _currentWorkspace = currentWorkspace;
        _currentUser = currentUser;
        _roleLookup = roleLookup;
        _projectMemberRepository = projectMemberRepository;
        _planRepository = planRepository;
        _authorizationService = authorizationService;
        _asyncExecuter = asyncExecuter;
        _workspaceSettings = workspaceSettings;
        _aiSettings = aiSettings;
        _subscriptions = subscriptions;
    }

    /// <param name="zoneHint">The time zone the request names (an IANA or Windows id), or null.</param>
    public virtual async Task<AiCallerContext> CreateAsync(string? zoneHint = null)
    {
        var workspaceId = _currentWorkspace.GetId();
        var userId = _currentUser.GetId();

        var role = WorkspaceRoles.Normalize(_currentWorkspace.Role) ?? await _roleLookup.FindRoleAsync(workspaceId, userId);

        var projectRoles = await _asyncExecuter.ToListAsync(
            (await _projectMemberRepository.GetQueryableAsync())
                .Where(m => m.WorkspaceId == workspaceId && m.UserId == userId)
                .Select(m => m.Role));
        var isClientOnly = role == WorkspaceRoles.Member && projectRoles.Count > 0 &&
                           projectRoles.All(TrackerRoles.IsClientProjectRole);
        var hasWorkerMembership = projectRoles.Any(r => !TrackerRoles.IsClientProjectRole(r));

        return new AiCallerContext(
            workspaceId,
            userId,
            role,
            isClientOnly,
            hasWorkerMembership,
            CanViewAll: await _authorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll),
            CanReport: await _authorizationService.IsGrantedAsync(TrackerPermissions.Reporting.Default),
            CanManageTeam: await _authorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage),
            CanEditProjects: await _authorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit),
            TimeZone: await ResolveZoneAsync(zoneHint),
            Policy: await _aiSettings.GetPolicyAsync(workspaceId),
            DisclosureVersion: _aiSettings.CurrentDisclosureVersion,
            PlanName: await FindPlanNameAsync());
    }

    private async Task<string> ResolveZoneAsync(string? zoneHint)
    {
        if (TimeZoneIds.TryNormalize(zoneHint, out var requested))
        {
            return requested;
        }

        return TimeZoneIds.TryNormalize(await _workspaceSettings.GetEffectiveTimeZoneAsync(), out var effective)
            ? effective
            : "UTC";
    }

    private async Task<string> FindPlanNameAsync()
    {
        var subscription = await _subscriptions.FindCurrentAsync();
        var plan = subscription == null ? null : await _planRepository.FindAsync(subscription.PlanId);
        return string.IsNullOrWhiteSpace(plan?.Name) ? AiCallerContext.DefaultPlanName : plan.Name;
    }
}
