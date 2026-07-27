using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Identity;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Users;

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

    public TeamAppService(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<Notification, Guid> notificationRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        InvoiceGenerator invoiceGenerator,
        IdentityUserManager userManager)
    {
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
        _notificationRepository = notificationRepository;
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _invoiceGenerator = invoiceGenerator;
        _userManager = userManager;
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
