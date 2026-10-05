using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.PermissionManagement;
using Volo.Abp.Uow;
using Volo.Abp.Users;
using Volo.Abp.Validation;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The workspace team: project rosters, adding and inviting people, roles and hourly rates, password resets and
/// removal from the workspace. Everything that changes the team needs Tracker.Team.Manage, which managers hold too, so
/// the owner-only rules are enforced here: only workspace owners (Identity "admin" role) grant Admin/Manager, set or
/// change hourly rates, touch an owner's memberships, bring back someone removed from the workspace, or reset or remove
/// an account with rights beyond the caller's; and nobody changes their own role or rate. Changes that can take a seat
/// are serialized per workspace (<see cref="SeatCounter.LockAsync"/>) and committed before the next one counts seats.
/// </summary>
[Authorize]
public class TeamAppService : TrackerAppService, ITeamAppService
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly SeatCounter _seatCounter;
    private readonly IdentityUserManager _userManager;
    private readonly IIdentityUserRepository _userRepository;
    private readonly IOptions<IdentityOptions> _identityOptions;
    private readonly ProjectMembershipManager _memberships;
    private readonly ISubscriptionAccessChecker _subscriptionAccess;
    private readonly IPermissionManager _permissionManager;

    public TeamAppService(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<Notification, Guid> notificationRepository,
        SeatCounter seatCounter,
        IdentityUserManager userManager,
        IIdentityUserRepository userRepository,
        IOptions<IdentityOptions> identityOptions,
        ProjectMembershipManager memberships,
        ISubscriptionAccessChecker subscriptionAccess,
        IPermissionManager permissionManager)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _notificationRepository = notificationRepository;
        _seatCounter = seatCounter;
        _userManager = userManager;
        _userRepository = userRepository;
        _identityOptions = identityOptions;
        _memberships = memberships;
        _subscriptionAccess = subscriptionAccess;
        _permissionManager = permissionManager;
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

    /// <remarks>An owner may also bring back someone removed from the workspace (a deactivated account): their account
    /// is reactivated with the password it had; a new one-time password is a <see cref="ResetPasswordAsync"/> away.
    /// That is the way back for an account whose email is not verified, which an invite never attaches to.</remarks>
    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input)
    {
        await using var seatLock = await _seatCounter.LockAsync();
        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);

        // The project must exist in the current tenant (throws EntityNotFoundException otherwise).
        var project = await _projectRepository.GetAsync(input.ProjectId);
        await EnsureSubscriptionUsableAsync();

        // Tenant-filtered: a user id of another workspace or a deleted user is refused, and so is a deactivated one
        // unless an owner brings them back.
        var user = await _userRepository.FindAsync(input.UserId);
        if (user == null || (!user.IsActive && !await IsOwnerAsync(CurrentUser.GetId())))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvalidUser)
                .WithData("userId", input.UserId);
        }

        var role = ProjectMemberRoles.Normalize(input.Role);
        await EnsureNotMemberAsync(project.Id, user.Id);
        var hourlyRate = await EnsureMayAssignAsync(project.Id, user.Id, role, input.HourlyRate);
        await EnsureSeatAvailableAsync(user.Id, role);

        if (!user.IsActive)
        {
            user.SetIsActive(true);
            (await _userManager.UpdateAsync(user)).CheckErrors();
            Logger.LogInformation("User {UserId} was brought back to the workspace by {CallerId}.", user.Id, CurrentUser.GetId());
        }

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

        var project = await _projectRepository.GetAsync(input.ProjectId);
        await EnsureSubscriptionUsableAsync();

        var email = input.Email.Trim();
        var role = ProjectMemberRoles.Normalize(input.Role);
        var user = await _userManager.FindByEmailAsync(email);
        string? initialPassword = null;
        var created = false;
        decimal hourlyRate;

        if (user != null)
        {
            // Users can edit their own email, so an unverified address proves nothing: attaching the invite to such an
            // account would let anyone claim a membership (and a Manager role) meant for the real owner of the address.
            if (!user.EmailConfirmed)
            {
                throw new AbpValidationException(new List<ValidationResult>
                {
                    new("An account with this email address already exists, but the address is not verified. " +
                        "Add that person from the team list instead.", new[] { nameof(InviteMemberDto.Email) })
                });
            }

            await EnsureNotMemberAsync(project.Id, user.Id);
            hourlyRate = await EnsureMayAssignAsync(project.Id, user.Id, role, input.HourlyRate);
            await EnsureSeatAvailableAsync(user.Id, role);

            if (!user.IsActive)
            {
                // Removed from the workspace earlier: inviting them again brings the account back, with a fresh
                // one-time password (the old one may be known to others by now).
                user.SetIsActive(true);
                initialPassword = await SetOneTimePasswordAsync(user);
            }
        }
        else
        {
            hourlyRate = await EnsureMayAssignAsync(project.Id, null, role, input.HourlyRate);
            await EnsureSeatAvailableAsync(null, role);

            // No email delivery is configured, so the one-time initial password is returned to the (Team.Manage)
            // caller, who shares it out-of-band; the invitee must change it at the first sign-in.
            initialPassword = GenerateInitialPassword();
            user = new IdentityUser(GuidGenerator.Create(), await CreateUserNameAsync(email), email, CurrentTenant.Id);
            // The address was entered by the workspace's manager, which is the verification this workspace has. The
            // user changing it later (profile) clears this again, so a re-invite never attaches to an edited address.
            user.SetEmailConfirmed(true);
            user.SetShouldChangePasswordOnNextLogin(true);

            await _identityOptions.SetAsync();
            (await _userManager.CreateAsync(user, initialPassword)).CheckErrors();
            created = true;
        }

        var member = await _memberships.EnrollAsync(project.Id, user.Id, role, hourlyRate);
        await NotifyAddedAsync(user.Id, project);
        await uow.CompleteAsync();

        return new InviteMemberResultDto
        {
            Member = ToDto(member, user, includePrivateData: true),
            UserCreated = created,
            InitialPassword = initialPassword
        };
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
        var canSeeEveryone =
            await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage) ||
            await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll);

        List<IdentityUser> users;
        if (canSeeEveryone)
        {
            // Tenant-filtered by ABP: only this workspace's users.
            users = await _userRepository.GetListAsync(sorting: nameof(IdentityUser.UserName));
        }
        else
        {
            var me = await _userRepository.FindAsync(CurrentUser.GetId());
            users = me == null ? new List<IdentityUser>() : new List<IdentityUser> { me };
        }

        if (users.Count == 0)
        {
            return new ListResultDto<TeamMemberDto>(new List<TeamMemberDto>());
        }

        var userIds = users.Select(u => u.Id).ToList();

        var roleNames = (await _userRepository.GetRoleNamesAsync(userIds))
            .ToDictionary(r => r.Id, r => r.RoleNames ?? Array.Empty<string>());

        var memberships = (await _memberRepository.GetListAsync(m => userIds.Contains(m.UserId)))
            .GroupBy(m => m.UserId)
            .ToDictionary(g => g.Key, g => g.ToList());

        bool HasRole(Guid userId, string role) =>
            roleNames.TryGetValue(userId, out var names) &&
            names.Any(n => string.Equals(n, role, StringComparison.OrdinalIgnoreCase));

        var items = users
            .Select(u =>
            {
                var userMemberships = memberships.GetValueOrDefault(u.Id) ?? new List<ProjectMember>();
                return new TeamMemberDto
                {
                    UserId = u.Id,
                    UserName = u.UserName,
                    Name = u.Name,
                    Surname = u.Surname,
                    Email = u.Email,
                    IsActive = u.IsActive,
                    IsManager = HasRole(u.Id, TrackerRoles.Manager),
                    IsOwner = HasRole(u.Id, TrackerRoles.Admin),
                    IsClient = userMemberships.Count > 0 && userMemberships.All(m => ProjectMemberRoles.IsClient(m.Role)),
                    ProjectIds = userMemberships.Select(m => m.ProjectId).Distinct().ToList()
                };
            })
            .ToList();

        return new ListResultDto<TeamMemberDto>(items);
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
    /// <remarks><paramref name="userId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?userId=</c>, the route of the shared contract) instead of a <c>/{userId}</c> route segment.</remarks>
    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ResetPasswordResultDto> ResetPasswordAsync([Optional] Guid userId)
    {
        var user = await GetManageableUserAsync(userId);

        var password = await SetOneTimePasswordAsync(user);

        // A forgotten password often comes with a locked-out account: let them in again. (Identity refuses to set a
        // lockout end for an account whose lockout is disabled, and such an account is never locked out anyway.)
        if (await _userManager.IsLockedOutAsync(user))
        {
            (await _userManager.SetLockoutEndDateAsync(user, null)).CheckErrors();
        }

        (await _userManager.ResetAccessFailedCountAsync(user)).CheckErrors();

        Logger.LogInformation("User {UserId} got a new one-time password from {CallerId}.", user.Id, CurrentUser.GetId());
        return new ResetPasswordResultDto { InitialPassword = password };
    }

    /// <inheritdoc />
    /// <remarks><paramref name="userId"/> is optional only so that ABP keeps it in the query string
    /// (<c>?userId=</c>, the route of the shared contract) instead of a <c>/{userId}</c> route segment.</remarks>
    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task RemoveFromWorkspaceAsync([Optional] Guid userId)
    {
        var user = await GetManageableUserAsync(userId, ownersAllowed: false);

        // A seat is a member with a project membership: removing them all frees it. The manager role goes in any case,
        // however it was given, so a later re-invite does not bring it back.
        var removed = await _memberships.RemoveAllOfUserAsync(user.Id);

        user.SetIsActive(false);
        (await _userManager.UpdateAsync(user)).CheckErrors();
        // Invalidates what the security stamp protects (e.g. password-reset tokens issued before the removal).
        (await _userManager.UpdateSecurityStampAsync(user)).CheckErrors();

        Logger.LogInformation("User {UserId} was removed from the workspace ({Count} membership(s)) by {CallerId}.",
            user.Id, removed, CurrentUser.GetId());
    }

    /// <summary>
    /// The target of a reset or removal: an existing user of this workspace other than the caller. An owner can only be
    /// targeted by an owner (reset), or not at all (<paramref name="ownersAllowed"/> false). Anyone else may only target
    /// an account whose rights do not go beyond their own (see <see cref="EnsureRightsWithinCallersAsync"/>).
    /// </summary>
    private async Task<IdentityUser> GetManageableUserAsync(Guid userId, bool ownersAllowed = true)
    {
        if (userId == CurrentUser.GetId())
        {
            throw new BusinessException(TrackerDomainErrorCodes.CannotModifySelf);
        }

        var user = await _userRepository.FindAsync(userId);
        if (user == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvalidUser)
                .WithData("userId", userId);
        }

        var callerIsOwner = await IsOwnerAsync(CurrentUser.GetId());
        if (await IsOwnerAsync(user.Id) && (!ownersAllowed || !callerIsOwner))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                .WithData("userId", userId);
        }

        if (!callerIsOwner)
        {
            await EnsureRightsWithinCallersAsync(user);
        }

        return user;
    }

    /// <summary>
    /// Refuses (<c>Tracker:RoleNotAllowed</c>) a target that holds a permission the caller does not — through any role
    /// or granted to them directly. Resetting someone's password hands the caller a way into their account, and removing
    /// them takes it from them: a manager must not do either to an account the owner gave more rights (billing, project
    /// deletion, identity administration, …). Both sides are read from the permission store, not from tokens.
    /// </summary>
    private async Task EnsureRightsWithinCallersAsync(IdentityUser target)
    {
        var targetPermissions = await GrantedPermissionsAsync(target.Id);
        if (targetPermissions.Count == 0)
        {
            return;
        }

        var callerPermissions = await GrantedPermissionsAsync(CurrentUser.GetId());
        var beyondCaller = targetPermissions.Where(name => !callerPermissions.Contains(name)).ToList();
        if (beyondCaller.Count > 0)
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                .WithData("userId", target.Id)
                .WithData("permissions", string.Join(", ", beyondCaller));
        }
    }

    /// <summary>Every permission granted to the user in the current workspace, through their roles or directly.</summary>
    private async Task<HashSet<string>> GrantedPermissionsAsync(Guid userId)
    {
        var permissions = await _permissionManager.GetAllForUserAsync(userId);
        return permissions.Where(p => p.IsGranted).Select(p => p.Name).ToHashSet(StringComparer.Ordinal);
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

    /// <summary>Workspace owners hold the Identity "admin" role (read from the store, not from a possibly stale token).</summary>
    private async Task<bool> IsOwnerAsync(Guid userId)
    {
        var roles = await _userRepository.GetRoleNamesAsync(userId);
        return roles.Any(r => string.Equals(r, TrackerRoles.Admin, StringComparison.OrdinalIgnoreCase));
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
            CurrentTenant.Id,
            userId,
            $"You were added to project '{project.Title}'."
        ));
    }

    /// <summary>Replaces the user's password with a new one-time password they must change at the next sign-in (the
    /// token endpoint then answers the password grant with <c>invalid_grant</c> / <c>ShouldChangePasswordOnNextLogin</c>
    /// and a change-password token). Saves the user, including any other pending change such as re-activation.</summary>
    private async Task<string> SetOneTimePasswordAsync(IdentityUser user)
    {
        var password = GenerateInitialPassword();
        user.SetShouldChangePasswordOnNextLogin(true);

        await _identityOptions.SetAsync();
        if (await _userManager.HasPasswordAsync(user))
        {
            (await _userManager.RemovePasswordAsync(user)).CheckErrors();
        }

        // Validates against the password policy and saves the user.
        (await _userManager.AddPasswordAsync(user, password)).CheckErrors();

        return password;
    }

    /// <summary>
    /// A user name for an invited person: their email when Identity accepts it as a user name (as before). Otherwise
    /// (e.g. o'brien@acme.com — an apostrophe is not a user-name character, so the invite used to fail) the allowed
    /// characters of the address's local part, without '@' so it can never equal somebody else's email, made unique
    /// with a number. People sign in with their email address either way: the token endpoint resolves it.
    /// </summary>
    private async Task<string> CreateUserNameAsync(string email)
    {
        var allowed = _userManager.Options.User.AllowedUserNameCharacters;
        if (InviteUserNames.IsAllowed(email, allowed))
        {
            return email;
        }

        var stem = InviteUserNames.Stem(email, allowed);
        for (var attempt = 1; ; attempt++)
        {
            var candidate = InviteUserNames.Candidate(stem, attempt);
            if (await _userManager.FindByNameAsync(candidate) == null)
            {
                return candidate;
            }
        }
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

    /// <summary>Random 20-character one-time password covering every complexity class (upper, lower, digit, symbol)
    /// with 64 bits of cryptographically secure randomness.</summary>
    private static string GenerateInitialPassword()
    {
        return "Dt1!" + RandomNumberGenerator.GetHexString(16, lowercase: true);
    }
}
