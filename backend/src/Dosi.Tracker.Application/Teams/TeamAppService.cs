using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Volo.Abp.Users;
using Volo.Abp.Validation;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The current workspace's team: project rosters, adding and inviting people, roles and hourly rates, and removal from
/// the workspace. Everything that changes the team needs Tracker.Team.Manage, which managers hold too, so the owner-only
/// rules are enforced here: only workspace owners grant Admin/Manager, set or change hourly rates, change workspace roles
/// or touch an owner; and nobody changes their own role or rate. Changes that can take a seat are serialized per
/// workspace (<see cref="SeatCounter.LockAsync"/>) and committed before the next one counts seats.
/// </summary>
[Authorize]
public class TeamAppService : TrackerAppService, ITeamAppService
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly SeatCounter _seatCounter;
    private readonly IdentityUserManager _userManager;
    private readonly IIdentityUserRepository _userRepository;
    private readonly ProjectMembershipManager _memberships;
    private readonly WorkspaceMembershipManager _workspaceMemberships;
    private readonly WorkspaceJoiner _joiner;
    private readonly ISubscriptionAccessChecker _subscriptionAccess;

    public TeamAppService(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<Notification, Guid> notificationRepository,
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IRepository<WorkspaceInvitation, Guid> invitationRepository,
        SeatCounter seatCounter,
        IdentityUserManager userManager,
        IIdentityUserRepository userRepository,
        ProjectMembershipManager memberships,
        WorkspaceMembershipManager workspaceMemberships,
        WorkspaceJoiner joiner,
        ISubscriptionAccessChecker subscriptionAccess)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _notificationRepository = notificationRepository;
        _workspaceMemberRepository = workspaceMemberRepository;
        _invitationRepository = invitationRepository;
        _seatCounter = seatCounter;
        _userManager = userManager;
        _userRepository = userRepository;
        _memberships = memberships;
        _workspaceMemberships = workspaceMemberships;
        _joiner = joiner;
        _subscriptionAccess = subscriptionAccess;
    }

    public async Task<ListResultDto<ProjectMemberDto>> GetProjectMembersAsync(Guid projectId)
    {
        var canManage = await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage);

        if (!canManage)
        {
            // Regular members may only view the roster of a project they belong to,
            // and never see anyone's pay rate or email.
            var userId = CurrentUser.GetId();
            var isMember = await _memberRepository.AnyAsync(
                m => m.ProjectId == projectId && m.UserId == userId);
            if (!isMember)
            {
                throw new AbpAuthorizationException("You are not a member of this project.");
            }
        }

        var members = (await _memberRepository.GetListAsync(m => m.ProjectId == projectId))
            .OrderBy(m => m.CreationTime)
            .ToList();
        var users = await GetUsersByIdAsync(members.Select(m => m.UserId));

        return new ListResultDto<ProjectMemberDto>(members
            .Select(m => ToDto(m, users.GetValueOrDefault(m.UserId), includePrivateData: canManage))
            .ToList());
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input)
    {
        await using var seatLock = await _seatCounter.LockAsync();
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);

        var project = await _projectRepository.GetAsync(input.ProjectId);
        await EnsureSubscriptionUsableAsync();

        // Only an active member of this workspace with an active account can join its projects.
        var user = await _userRepository.FindAsync(input.UserId);
        if (user == null || !user.IsActive || await _workspaceMemberships.FindActiveAsync(WorkspaceId, user.Id) == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvalidUser)
                .WithData("userId", input.UserId);
        }

        var role = ProjectMemberRoles.Normalize(input.Role);
        await EnsureNotMemberAsync(project.Id, user.Id);
        var hourlyRate = await EnsureMayAssignAsync(project.Id, user.Id, role, input.HourlyRate);
        await EnsureSeatAvailableAsync(user.Id, role);

        var member = await _memberships.EnrollAsync(project.Id, user.Id, role, hourlyRate);
        await NotifyAddedAsync(user.Id, project);
        await uow.CompleteAsync();

        return ToDto(member, user, includePrivateData: true);
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<InviteMemberResultDto> InviteMemberAsync(InviteMemberDto input)
    {
        await using var seatLock = await _seatCounter.LockAsync();
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);

        var workspaceId = WorkspaceId;
        var callerIsOwner = await IsOwnerAsync(CurrentUser.GetId());
        var workspaceRole = input.WorkspaceRole.IsNullOrWhiteSpace()
            ? WorkspaceRoles.Member
            : WorkspaceRoles.Normalize(input.WorkspaceRole) ?? throw new AbpValidationException(new List<ValidationResult>
            {
                new("The workspace role must be owner, manager or member.", new[] { nameof(InviteMemberDto.WorkspaceRole) })
            });
        if (workspaceRole != WorkspaceRoles.Member && !callerIsOwner)
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed).WithData("role", workspaceRole);
        }

        Project? project = null;
        var role = ProjectMemberRoles.Normalize(input.Role.IsNullOrWhiteSpace() ? ProjectMemberRoles.Worker : input.Role);
        if (input.ProjectId.HasValue)
        {
            project = await _projectRepository.GetAsync(input.ProjectId.Value);
        }

        await EnsureSubscriptionUsableAsync();

        var email = input.Email.Trim();
        var user = await _userManager.FindByEmailAsync(email);
        if (user != null)
        {
            if (!user.IsActive)
            {
                throw new BusinessException(TrackerDomainErrorCodes.InvalidUser).WithData("email", email);
            }

            var existing = await _workspaceMemberships.FindActiveAsync(workspaceId, user.Id);
            if (existing != null && project == null)
            {
                throw new BusinessException(TrackerDomainErrorCodes.AlreadyWorkspaceMember).WithData("email", email);
            }

            if (existing is { IsOwner: true } && !callerIsOwner)
            {
                throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed).WithData("userId", user.Id);
            }

            var hourlyRate = 0m;
            if (project != null)
            {
                await EnsureNotMemberAsync(project.Id, user.Id);
                hourlyRate = await EnsureMayAssignAsync(project.Id, user.Id, role, input.HourlyRate);
            }

            var (member, projectMember) = await _joiner.JoinAsync(user.Id, workspaceRole, project?.Id, role, hourlyRate);
            await uow.CompleteAsync();

            return new InviteMemberResultDto
            {
                Added = true,
                WorkspaceRole = member.Role,
                Member = projectMember == null ? null : ToDto(projectMember, user, includePrivateData: true)
            };
        }

        var rate = project == null ? 0m : await EnsureMayAssignAsync(project.Id, null, role, input.HourlyRate);
        var normalizedEmail = WorkspaceInvitation.NormalizeEmail(email);
        var stale = await _invitationRepository.GetListAsync(i =>
            i.WorkspaceId == workspaceId && i.NormalizedEmail == normalizedEmail && i.AcceptedAt == null);
        if (stale.Count > 0)
        {
            await _invitationRepository.DeleteManyAsync(stale);
        }

        var token = WorkspaceInvitation.NewToken();
        var invitation = new WorkspaceInvitation(
            GuidGenerator.Create(),
            workspaceId,
            email,
            WorkspaceJoiner.EffectiveRole(workspaceRole, project?.Id, role),
            project?.Id,
            project == null ? null : role,
            project == null ? null : rate,
            WorkspaceInvitation.HashToken(token),
            Clock.Now.AddDays(WorkspaceConsts.InvitationLifetimeDays));
        await _invitationRepository.InsertAsync(invitation, autoSave: true);
        await uow.CompleteAsync();

        return new InviteMemberResultDto
        {
            Added = false,
            WorkspaceRole = invitation.Role,
            InvitationId = invitation.Id,
            InvitePath = "/invite/" + token,
            ExpiresAt = invitation.ExpiresAt
        };
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ListResultDto<WorkspaceInvitationDto>> GetInvitationsAsync()
    {
        var workspaceId = WorkspaceId;
        var now = Clock.Now;
        var invitations = await _invitationRepository.GetListAsync(i =>
            i.WorkspaceId == workspaceId && i.AcceptedAt == null && i.ExpiresAt > now);

        return new ListResultDto<WorkspaceInvitationDto>(invitations
            .OrderByDescending(i => i.CreationTime)
            .Select(i => new WorkspaceInvitationDto
            {
                Id = i.Id,
                Email = i.Email,
                Role = i.Role,
                ProjectId = i.ProjectId,
                ProjectRole = i.ProjectRole,
                ExpiresAt = i.ExpiresAt,
                CreationTime = i.CreationTime
            })
            .ToList());
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task DeleteInvitationAsync(Guid id)
    {
        var workspaceId = WorkspaceId;
        var invitation = await _invitationRepository.FirstOrDefaultAsync(i => i.Id == id && i.WorkspaceId == workspaceId)
                         ?? throw new EntityNotFoundException(typeof(WorkspaceInvitation), id);
        await _invitationRepository.DeleteAsync(invitation);
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task RemoveMemberAsync(Guid memberId)
    {
        var member = await _memberRepository.FindAsync(memberId);
        if (member == null)
        {
            throw new EntityNotFoundException(typeof(ProjectMember), memberId);
        }

        var callerIsOwner = await IsOwnerAsync(CurrentUser.GetId());
        if (member.UserId == CurrentUser.GetId() && !callerIsOwner)
        {
            throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
        }

        if (!callerIsOwner && await IsOwnerAsync(member.UserId))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                .WithData("userId", member.UserId);
        }

        await _memberships.RemoveAsync(member);
    }

    public async Task<ListResultDto<TeamMemberDto>> GetMembersAsync()
    {
        var workspaceId = WorkspaceId;
        var canSeeEveryone =
            await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage) ||
            await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll);

        var me = CurrentUser.GetId();
        var workspaceMembers = await _workspaceMemberRepository.GetListAsync(m =>
            m.WorkspaceId == workspaceId && m.Status == WorkspaceMemberStatus.Active && (canSeeEveryone || m.UserId == me));
        if (workspaceMembers.Count == 0)
        {
            return new ListResultDto<TeamMemberDto>(new List<TeamMemberDto>());
        }

        var userIds = workspaceMembers.Select(m => m.UserId).ToList();
        var users = await GetUsersByIdAsync(userIds);
        var memberships = (await _memberRepository.GetListAsync(m => userIds.Contains(m.UserId)))
            .GroupBy(m => m.UserId)
            .ToDictionary(g => g.Key, g => g.ToList());

        var items = workspaceMembers
            .Where(m => users.ContainsKey(m.UserId))
            .Select(m => ToTeamMemberDto(m, users[m.UserId], memberships.GetValueOrDefault(m.UserId) ?? new List<ProjectMember>()))
            .OrderBy(m => m.UserName, StringComparer.OrdinalIgnoreCase)
            .ToList();

        return new ListResultDto<TeamMemberDto>(items);
    }

    private static TeamMemberDto ToTeamMemberDto(WorkspaceMember member, IdentityUser user, List<ProjectMember> memberships)
    {
        return new TeamMemberDto
        {
            UserId = user.Id,
            UserName = user.UserName,
            Name = user.Name,
            Surname = user.Surname,
            Email = user.Email,
            IsActive = user.IsActive,
            Role = member.Role,
            IsManager = member.Role == WorkspaceRoles.Manager,
            IsOwner = member.Role == WorkspaceRoles.Owner,
            IsClient = memberships.Count > 0 && memberships.All(m => ProjectMemberRoles.IsClient(m.Role)),
            JoinedAt = member.JoinedAt,
            ProjectIds = memberships.Select(m => m.ProjectId).Distinct().ToList()
        };
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ProjectMemberDto> UpdateMemberAsync(Guid projectId, Guid userId, UpdateProjectMemberDto input)
    {
        // Turning a client into a tracked member takes a seat (see AddMemberAsync).
        await using var seatLock = await _seatCounter.LockAsync();
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);

        var member = await _memberRepository.FirstOrDefaultAsync(
            m => m.ProjectId == projectId && m.UserId == userId);
        if (member == null)
        {
            throw new EntityNotFoundException(typeof(ProjectMember), $"{projectId}/{userId}");
        }

        var role = ProjectMemberRoles.Normalize(input.Role);
        var roleChanged = !ProjectMemberRoles.AreSame(member.Role, role);
        var rateChanged = member.HourlyRate != input.HourlyRate;

        if (roleChanged || rateChanged)
        {
            if (userId == CurrentUser.GetId())
            {
                throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
            }

            if (!await IsOwnerAsync(CurrentUser.GetId()))
            {
                if (await IsOwnerAsync(userId) ||
                    rateChanged ||
                    (roleChanged && ProjectMemberRoles.IsManagerLevel(role)))
                {
                    throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                        .WithData("role", role);
                }
            }

            // A client who becomes a tracked member needs a seat from now on — which a workspace whose subscription
            // lapsed cannot take (it cannot grow).
            if (ProjectMemberRoles.IsClient(member.Role) && !ProjectMemberRoles.IsClient(role))
            {
                await EnsureSubscriptionUsableAsync();
                await EnsureSeatAvailableAsync(userId, role);
            }

            await _memberships.ChangeAsync(member, role, input.HourlyRate);
        }

        var dto = ToDto(member, await _userRepository.FindAsync(userId), includePrivateData: true);
        await uow.CompleteAsync();
        return dto;
    }

    /// <inheritdoc />
    /// <remarks><paramref name="userId"/> is optional only so that ABP keeps it in the query string.</remarks>
    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<TeamMemberDto> UpdateWorkspaceRoleAsync([Optional] Guid userId, string role)
    {
        var newRole = WorkspaceRoles.Normalize(role) ?? throw new AbpValidationException(new List<ValidationResult>
        {
            new("The role must be owner, manager or member.", new[] { nameof(role) })
        });

        if (!await IsOwnerAsync(CurrentUser.GetId()))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed).WithData("role", newRole);
        }

        if (userId == CurrentUser.GetId())
        {
            throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
        }

        using var uow = UnitOfWorkManager.Begin(new Volo.Abp.Uow.AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        await using var ownerLock = await _workspaceMemberships.LockOwnersAsync(WorkspaceId);

        var member = await _workspaceMemberships.FindActiveAsync(WorkspaceId, userId)
                     ?? throw new BusinessException(TrackerDomainErrorCodes.InvalidUser).WithData("userId", userId);
        await _workspaceMemberships.ChangeRoleAsync(member, newRole);

        var user = await _userRepository.GetAsync(userId);
        var memberships = await _memberRepository.GetListAsync(m => m.UserId == userId);
        await uow.CompleteAsync();

        return ToTeamMemberDto(member, user, memberships);
    }

    /// <inheritdoc />
    /// <remarks><paramref name="userId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?userId=</c>, the route of the shared contract) instead of a <c>/{userId}</c> route segment.</remarks>
    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task RemoveFromWorkspaceAsync([Optional] Guid userId)
    {
        if (userId == CurrentUser.GetId())
        {
            throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
        }

        var member = await _workspaceMemberships.FindActiveAsync(WorkspaceId, userId)
                     ?? throw new BusinessException(TrackerDomainErrorCodes.InvalidUser).WithData("userId", userId);

        var callerIsOwner = await IsOwnerAsync(CurrentUser.GetId());
        if (member.IsOwner || (member.Role == WorkspaceRoles.Manager && !callerIsOwner))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed).WithData("userId", userId);
        }

        // Project memberships occupy seats: removing them all frees the person's seat. The account is untouched.
        var removed = await _memberships.RemoveAllOfUserAsync(userId);
        await _workspaceMemberships.RemoveAsync(member);

        Logger.LogInformation("User {UserId} was removed from workspace {WorkspaceId} ({Count} membership(s)) by {CallerId}.",
            userId, member.WorkspaceId, removed, CurrentUser.GetId());
    }

    /// <summary>
    /// The caller's right to give <paramref name="targetUserId"/> (null: a person without an account yet) this role on the
    /// project, and the hourly rate the membership starts with. Nobody assigns themselves a manager-level role or a new
    /// rate, and only owners grant Admin/Manager, set a rate or manage an owner's memberships. Unless an owner sets the rate
    /// of someone else, a membership starts at the person's latest recorded rate on the project (0 if they never had one):
    /// removing and re-adding someone must not re-price their time. A rate equal to that recorded one is no change.
    /// </summary>
    private async Task<decimal> EnsureMayAssignAsync(Guid projectId, Guid? targetUserId, string role, decimal requestedRate)
    {
        var callerId = CurrentUser.GetId();
        var callerIsOwner = await IsOwnerAsync(callerId);
        var recordedRate = targetUserId.HasValue
            ? await _memberships.FindLatestRateAsync(projectId, targetUserId.Value) ?? 0m
            : 0m;

        // An owner's rate for someone else is what they ask for; otherwise 0 means "no rate given" and keeps the recorded one.
        var ownerSetsRate = callerIsOwner && targetUserId != callerId;
        var hourlyRate = ownerSetsRate || requestedRate != 0m ? requestedRate : recordedRate;
        var setsRoleOrRate = ProjectMemberRoles.IsManagerLevel(role) || hourlyRate != recordedRate;

        if (targetUserId == callerId && setsRoleOrRate)
        {
            throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
        }

        if (!callerIsOwner && (setsRoleOrRate || (targetUserId.HasValue && await IsOwnerAsync(targetUserId.Value))))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                .WithData("role", role);
        }

        return hourlyRate;
    }

    private Guid WorkspaceId => CurrentWorkspace.GetId();

    /// <summary>An active owner of the current workspace (read from the store, never from a token).</summary>
    private async Task<bool> IsOwnerAsync(Guid userId)
    {
        var member = await _workspaceMemberships.FindActiveAsync(WorkspaceId, userId);
        return member is { IsOwner: true };
    }

    private async Task EnsureNotMemberAsync(Guid projectId, Guid userId)
    {
        if (await _memberRepository.AnyAsync(m => m.ProjectId == projectId && m.UserId == userId))
        {
            throw new BusinessException(TrackerDomainErrorCodes.DuplicateProjectMember)
                .WithData("projectId", projectId)
                .WithData("userId", userId);
        }
    }

    /// <summary>A workspace whose subscription lapsed keeps its data readable but cannot add people.</summary>
    private async Task EnsureSubscriptionUsableAsync()
    {
        if (!await _subscriptionAccess.IsUsableAsync())
        {
            throw new BusinessException(TrackerDomainErrorCodes.SubscriptionInactive);
        }
    }

    /// <summary>
    /// Enforces the subscription plan's seat limit (<see cref="SeatCounter"/>, the definition the invoice and the host
    /// console use too): someone who already holds a seat can join more projects, and a Client needs no seat at all.
    /// </summary>
    private async Task EnsureSeatAvailableAsync(Guid? userId, string role)
    {
        if (ProjectMemberRoles.IsClient(role))
        {
            return;
        }

        await _seatCounter.CheckSeatAvailableAsync(userId);
    }

    private async Task NotifyAddedAsync(Guid userId, Project project)
    {
        // Let the member know they can start tracking.
        await _notificationRepository.InsertAsync(new Notification(
            GuidGenerator.Create(),
            WorkspaceId,
            userId,
            $"You were added to project '{project.Title}'."
        ));
    }

    private async Task<Dictionary<Guid, IdentityUser>> GetUsersByIdAsync(IEnumerable<Guid> userIds)
    {
        var ids = userIds.Distinct().ToList();
        if (ids.Count == 0)
        {
            return new Dictionary<Guid, IdentityUser>();
        }

        return (await _userRepository.GetListByIdsAsync(ids)).ToDictionary(u => u.Id);
    }

    /// <summary>The membership with the member's names; the rate and the email only for callers who manage the team.</summary>
    private static ProjectMemberDto ToDto(ProjectMember member, IdentityUser? user, bool includePrivateData)
    {
        return new ProjectMemberDto
        {
            Id = member.Id,
            CreationTime = member.CreationTime,
            CreatorId = member.CreatorId,
            ProjectId = member.ProjectId,
            UserId = member.UserId,
            Role = member.Role,
            HourlyRate = includePrivateData ? member.HourlyRate : 0m,
            UserName = user?.UserName ?? string.Empty,
            Name = user?.Name,
            Surname = user?.Surname,
            Email = includePrivateData ? user?.Email : null
        };
    }
}
