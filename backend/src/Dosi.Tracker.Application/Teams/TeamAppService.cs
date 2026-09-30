using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Users;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Teams;

[Authorize]
public class TeamAppService : TrackerAppService, ITeamAppService
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly InvoiceGenerator _invoiceGenerator;
    private readonly IdentityUserManager _userManager;
    private readonly IIdentityUserRepository _userRepository;
    private readonly ManagerRoleProvisioner _managerRoleProvisioner;

    public TeamAppService(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<Notification, Guid> notificationRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        InvoiceGenerator invoiceGenerator,
        IdentityUserManager userManager,
        IIdentityUserRepository userRepository,
        ManagerRoleProvisioner managerRoleProvisioner)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _notificationRepository = notificationRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceGenerator = invoiceGenerator;
        _userManager = userManager;
        _userRepository = userRepository;
        _managerRoleProvisioner = managerRoleProvisioner;
    }

    public async Task<ListResultDto<ProjectMemberDto>> GetProjectMembersAsync(Guid projectId)
    {
        var canManage = await AuthorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage);

        if (!canManage)
        {
            // Regular members may only view the roster of a project they belong to,
            // and never see anyone's pay rate.
            var userId = CurrentUser.GetId();
            var isMember = await _memberRepository.AnyAsync(
                m => m.ProjectId == projectId && m.UserId == userId);
            if (!isMember)
            {
                throw new AbpAuthorizationException("You are not a member of this project.");
            }
        }

        var members = await _memberRepository.GetListAsync(m => m.ProjectId == projectId);
        var dtos = ObjectMapper.Map<List<ProjectMember>, List<ProjectMemberDto>>(members);

        if (!canManage)
        {
            // HourlyRate is payroll data — managers only.
            foreach (var dto in dtos)
            {
                dto.HourlyRate = 0m;
            }
        }

        return new ListResultDto<ProjectMemberDto>(dtos);
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input)
    {
        // The project must exist in the current tenant (throws EntityNotFoundException otherwise).
        var project = await _projectRepository.GetAsync(input.ProjectId);

        var alreadyMember = await _memberRepository.AnyAsync(
            m => m.ProjectId == input.ProjectId && m.UserId == input.UserId);
        if (alreadyMember)
        {
            throw new BusinessException(TrackerDomainErrorCodes.DuplicateProjectMember)
                .WithData("projectId", input.ProjectId)
                .WithData("userId", input.UserId);
        }

        await EnsureSeatAvailableAsync(input.UserId);

        var member = new ProjectMember(
            GuidGenerator.Create(),
            CurrentTenant.Id,
            input.ProjectId,
            input.UserId,
            input.Role,
            input.HourlyRate
        );
        await _memberRepository.InsertAsync(member);

        if (TrackerRoles.IsManagerProjectRole(input.Role))
        {
            await GrantManagerRoleAsync(input.UserId);
        }

        // Let the member know they can start tracking.
        await _notificationRepository.InsertAsync(new Notification(
            GuidGenerator.Create(),
            CurrentTenant.Id,
            input.UserId,
            $"You were added to project '{project.Title}'."
        ));

        return ObjectMapper.Map<ProjectMember, ProjectMemberDto>(member);
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<InviteMemberResultDto> InviteMemberAsync(InviteMemberDto input)
    {
        var user = await _userManager.FindByEmailAsync(input.Email);
        var created = false;
        string? initialPassword = null;

        if (user == null)
        {
            // No email delivery is configured in this template, so the one-time
            // initial password is returned to the (Team.Manage) caller instead;
            // the invitee should change it on first sign-in.
            initialPassword = GenerateInitialPassword();
            user = new IdentityUser(GuidGenerator.Create(), input.Email, input.Email, CurrentTenant.Id);
            (await _userManager.CreateAsync(user, initialPassword)).CheckErrors();
            created = true;
        }

        var member = await AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = input.ProjectId,
            UserId = user.Id,
            Role = input.Role,
            HourlyRate = input.HourlyRate
        });

        return new InviteMemberResultDto
        {
            Member = member,
            UserCreated = created,
            InitialPassword = initialPassword
        };
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task RemoveMemberAsync(Guid memberId)
    {
        await _memberRepository.DeleteAsync(memberId);
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

        var projectIds = (await _memberRepository.GetListAsync(m => userIds.Contains(m.UserId)))
            .GroupBy(m => m.UserId)
            .ToDictionary(g => g.Key, g => g.Select(m => m.ProjectId).Distinct().ToList());

        bool HasRole(Guid userId, string role) =>
            roleNames.TryGetValue(userId, out var names) &&
            names.Any(n => string.Equals(n, role, StringComparison.OrdinalIgnoreCase));

        var items = users
            .Select(u => new TeamMemberDto
            {
                UserId = u.Id,
                UserName = u.UserName,
                Name = u.Name,
                Surname = u.Surname,
                Email = u.Email,
                IsActive = u.IsActive,
                IsManager = HasRole(u.Id, TrackerRoles.Manager),
                IsOwner = HasRole(u.Id, TrackerRoles.Admin),
                ProjectIds = projectIds.GetValueOrDefault(u.Id) ?? new List<Guid>()
            })
            .ToList();

        return new ListResultDto<TeamMemberDto>(items);
    }

    [Authorize(TrackerPermissions.Team.Manage)]
    public async Task<ProjectMemberDto> UpdateMemberAsync(Guid projectId, Guid userId, UpdateProjectMemberDto input)
    {
        var member = await _memberRepository.FirstOrDefaultAsync(
            m => m.ProjectId == projectId && m.UserId == userId);
        if (member == null)
        {
            throw new EntityNotFoundException(typeof(ProjectMember), $"{projectId}/{userId}");
        }

        var wasManagerRole = TrackerRoles.IsManagerProjectRole(member.Role);
        member.Role = input.Role;
        member.HourlyRate = input.HourlyRate;
        await _memberRepository.UpdateAsync(member, autoSave: true);

        if (TrackerRoles.IsManagerProjectRole(input.Role))
        {
            await GrantManagerRoleAsync(userId);
        }
        else if (wasManagerRole)
        {
            await RevokeManagerRoleIfUnusedAsync(userId);
        }

        return ObjectMapper.Map<ProjectMember, ProjectMemberDto>(member);
    }

    /// <summary>Gives the user the Identity "manager" role (created on demand for tenants that predate it).
    /// A membership for a user id that has no Identity account is left as a plain membership.</summary>
    private async Task GrantManagerRoleAsync(Guid userId)
    {
        var user = await _userManager.FindByIdAsync(userId.ToString());
        if (user == null)
        {
            Logger.LogWarning("Cannot grant the '{Role}' role: user {UserId} does not exist.", TrackerRoles.Manager, userId);
            return;
        }

        await _managerRoleProvisioner.EnsureAsync();

        if (!await _userManager.IsInRoleAsync(user, TrackerRoles.Manager))
        {
            (await _userManager.AddToRoleAsync(user, TrackerRoles.Manager)).CheckErrors();
        }
    }

    /// <summary>After a demotion, drops the "manager" role unless the user is still Admin/Manager on another project.</summary>
    private async Task RevokeManagerRoleIfUnusedAsync(Guid userId)
    {
        var memberships = await _memberRepository.GetListAsync(m => m.UserId == userId);
        if (memberships.Any(m => TrackerRoles.IsManagerProjectRole(m.Role)))
        {
            return;
        }

        var user = await _userManager.FindByIdAsync(userId.ToString());
        if (user != null && await _userManager.IsInRoleAsync(user, TrackerRoles.Manager))
        {
            (await _userManager.RemoveFromRoleAsync(user, TrackerRoles.Manager)).CheckErrors();
        }
    }

    /// <summary>Random 20-char password covering every default complexity class.</summary>
    private static string GenerateInitialPassword()
    {
        return $"Dt1!{Guid.NewGuid():N}"[..20];
    }

    /// <summary>Enforces the subscription plan's seat limit. A user who is already a member
    /// of another project occupies a seat already and can always be added to more projects.</summary>
    private async Task EnsureSeatAvailableAsync(Guid userId)
    {
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(
            s => s.TenantId == CurrentTenant.Id);
        if (subscription == null)
        {
            return; // No subscription (e.g. host context) — nothing to enforce.
        }

        var plan = await _planRepository.FindAsync(subscription.PlanId);
        if (plan == null || plan.MaxSeats <= 0)
        {
            return;
        }

        var occupiesSeatAlready = await _memberRepository.AnyAsync(m => m.UserId == userId);
        if (occupiesSeatAlready)
        {
            return;
        }

        var seatsUsed = await _invoiceGenerator.CountSeatsAsync();
        if (seatsUsed >= plan.MaxSeats)
        {
            throw new BusinessException(TrackerDomainErrorCodes.SeatLimitReached)
                .WithData("maxSeats", plan.MaxSeats)
                .WithData("plan", plan.Name);
        }
    }
}
