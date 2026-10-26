using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Authorization;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Volo.Abp.Users;
using Volo.Abp.Validation;

namespace Dosi.Tracker.SaaS;

[Authorize]
public class WorkspaceAppService : TrackerAppService, IWorkspaceAppService
{
    public const string DefaultPlanName = Plan.FreePlanName;

    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<WorkspaceInvitation, Guid> _invitationRepository;
    private readonly WorkspaceManager _workspaceManager;
    private readonly WorkspaceMembershipManager _memberships;
    private readonly WorkspaceSettingsManager _workspaceSettings;
    private readonly TrackerAccountFactory _accounts;
    private readonly WorkspaceJoiner _joiner;
    private readonly ProjectMembershipManager _projectMemberships;
    private readonly SeatCounter _seatCounter;
    private readonly IdentityUserManager _userManager;
    private readonly IOptions<CaptureRetentionOptions> _captureRetentionOptions;
    private readonly IOptions<ActivityRetentionOptions> _activityRetentionOptions;

    public WorkspaceAppService(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        IRepository<Workspace, Guid> workspaceRepository,
        IRepository<WorkspaceMember, Guid> memberRepository,
        IRepository<WorkspaceInvitation, Guid> invitationRepository,
        WorkspaceManager workspaceManager,
        WorkspaceMembershipManager memberships,
        WorkspaceSettingsManager workspaceSettings,
        TrackerAccountFactory accounts,
        WorkspaceJoiner joiner,
        ProjectMembershipManager projectMemberships,
        SeatCounter seatCounter,
        IdentityUserManager userManager,
        IOptions<CaptureRetentionOptions> captureRetentionOptions,
        IOptions<ActivityRetentionOptions> activityRetentionOptions)
    {
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _workspaceRepository = workspaceRepository;
        _memberRepository = memberRepository;
        _invitationRepository = invitationRepository;
        _workspaceManager = workspaceManager;
        _memberships = memberships;
        _workspaceSettings = workspaceSettings;
        _accounts = accounts;
        _joiner = joiner;
        _projectMemberships = projectMemberships;
        _seatCounter = seatCounter;
        _userManager = userManager;
        _captureRetentionOptions = captureRetentionOptions;
        _activityRetentionOptions = activityRetentionOptions;
    }

    public async Task<SubscriptionDto?> GetCurrentSubscriptionAsync()
    {
        var workspaceId = CurrentWorkspace.GetId();
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == workspaceId);
        return subscription == null ? null : ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
    }

    /// <summary>Anonymous: the sign-up page offers the plans as the platform currently defines them.</summary>
    [AllowAnonymous]
    [WorkspaceNotRequired]
    public async Task<ListResultDto<PlanDto>> GetAvailablePlansAsync()
    {
        var plans = (await _planRepository.GetListAsync())
            .OrderBy(p => p.PricePerUser)
            .ThenBy(p => p.Name)
            .ToList();

        var dtos = ObjectMapper.Map<List<Plan>, List<PlanDto>>(plans);
        foreach (var dto in dtos)
        {
            dto.CreatorId = null;
            dto.LastModifierId = null;
            dto.DeleterId = null;
        }

        return new ListResultDto<PlanDto>(dtos);
    }

    /// <summary>Anonymous: the sign-up page and the web app hide plans and billing while this deployment does not bill.</summary>
    [AllowAnonymous]
    [WorkspaceNotRequired]
    public Task<BillingModeDto> GetBillingModeAsync()
    {
        return Task.FromResult(new BillingModeDto { Enabled = IsBillingEnabled });
    }

    [AllowAnonymous]
    [WorkspaceNotRequired]
    public async Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input)
    {
        await _accounts.EnsureEmailIsFreeAsync(input.Email);
        await _accounts.CheckPasswordAsync(input.Email, input.Password, nameof(RegisterWorkspaceDto.Password));

        if (!input.InviteToken.IsNullOrWhiteSpace())
        {
            return await RegisterWithInvitationAsync(input);
        }

        var plan = await GetSignUpPlanAsync(input.PlanId, input.PlanName);
        var name = input.WorkspaceName!.Trim();

        await using var nameLock = await _workspaceManager.LockNameAsync(name);
        using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);

        if (await _workspaceManager.IsNameTakenAsync(name))
        {
            throw WorkspaceManager.DuplicateName(name);
        }

        var user = await _accounts.CreateAsync(input.Email, input.Password, input.FirstName, input.Surname);
        var workspace = await _workspaceManager.CreateAsync(name, user.Id, plan);
        await uow.CompleteAsync();

        return new WorkspaceRegistrationResultDto
        {
            WorkspaceId = workspace.Id,
            WorkspaceName = workspace.Name,
            UserName = user.UserName,
            EmailConfirmationSent = await SendEmailConfirmationAsync(user)
        };
    }

    private async Task<WorkspaceRegistrationResultDto> RegisterWithInvitationAsync(RegisterWorkspaceDto input)
    {
        var invitation = await GetUsableInvitationAsync(input.InviteToken!);
        if (!invitation.IsFor(input.Email))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        if (await _workspaceManager.IsArchivedAsync(invitation.WorkspaceId))
        {
            throw new BusinessException(TrackerDomainErrorCodes.WorkspaceArchived);
        }

        using (CurrentWorkspace.Change(invitation.WorkspaceId))
        {
            await using var seatLock = await _seatCounter.LockAsync();
            using var uow = UnitOfWorkManager.Begin(isTransactional: true);

            var user = await _accounts.CreateAsync(input.Email, input.Password, input.FirstName, input.Surname);
            var workspace = await AcceptAsync(invitation, user.Id);
            await uow.CompleteAsync();

            return new WorkspaceRegistrationResultDto
            {
                WorkspaceId = workspace.Id,
                WorkspaceName = workspace.Name,
                UserName = user.UserName,
                EmailConfirmationSent = await SendEmailConfirmationAsync(user)
            };
        }
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<List<MyWorkspaceDto>> GetMineAsync()
    {
        var userId = CurrentUser.GetId();
        var members = await _memberRepository.GetQueryableAsync();
        var workspaces = await _workspaceRepository.GetQueryableAsync();

        var rows = await AsyncExecuter.ToListAsync(
            from m in members
            join w in workspaces on m.WorkspaceId equals w.Id
            where m.UserId == userId && m.Status == WorkspaceMemberStatus.Active
            select new { w.Id, w.Name, m.Role, w.IsArchived, w.ArchivedTime });

        var ids = rows.Select(r => r.Id).ToList();
        Dictionary<Guid, Guid> planIds;
        using (DataFilter.Disable<IWorkspaceScoped>())
        {
            planIds = (await _subscriptionRepository.GetListAsync(s => ids.Contains(s.WorkspaceId)))
                .GroupBy(s => s.WorkspaceId)
                .ToDictionary(g => g.Key, g => g.OrderByDescending(s => s.CreationTime).First().PlanId);
        }

        var plans = (await _planRepository.GetListAsync()).ToDictionary(p => p.Id, p => p.Name);

        return rows
            .Select(r => new MyWorkspaceDto
            {
                Id = r.Id,
                Name = r.Name,
                Role = r.Role,
                IsOwner = r.Role == WorkspaceRoles.Owner,
                IsArchived = r.IsArchived,
                ArchivedTime = r.ArchivedTime,
                PlanName = planIds.TryGetValue(r.Id, out var planId) ? plans.GetValueOrDefault(planId) : null
            })
            .OrderBy(w => w.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<CreatedWorkspaceDto> CreateAsync(CreateWorkspaceDto input)
    {
        var plan = await GetSignUpPlanAsync(input.PlanId, null);
        var name = input.Name.Trim();

        await using var nameLock = await _workspaceManager.LockNameAsync(name);
        using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
        var workspace = await _workspaceManager.CreateAsync(name, CurrentUser.GetId(), plan);
        await uow.CompleteAsync();

        return new CreatedWorkspaceDto { Id = workspace.Id, Name = workspace.Name };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task LeaveAsync(Guid id)
    {
        var userId = CurrentUser.GetId();

        using var uow = UnitOfWorkManager.Begin(new Volo.Abp.Uow.AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        await using var ownerLock = await _memberships.LockOwnersAsync(id);

        var member = await _memberships.FindActiveAsync(id, userId)
                     ?? throw new BusinessException(TrackerDomainErrorCodes.NotWorkspaceMember);

        using (CurrentWorkspace.Change(id))
        {
            await _memberships.RemoveAsync(member);
            await _projectMemberships.RemoveAllOfUserAsync(userId);
        }

        await uow.CompleteAsync();
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceDto> ArchiveAsync(Guid id)
    {
        var userId = CurrentUser.GetId();
        var workspace = await _workspaceManager.ArchiveAsync(id, userId);

        return new WorkspaceDto
        {
            Id = workspace.Id,
            Name = workspace.Name,
            IsArchived = workspace.IsArchived,
            ArchivedTime = workspace.ArchivedTime,
            ArchivedBy = workspace.ArchivedBy
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceDto> RestoreAsync(Guid id)
    {
        var userId = CurrentUser.GetId();
        var workspace = await _workspaceManager.RestoreAsync(id, userId);

        return new WorkspaceDto
        {
            Id = workspace.Id,
            Name = workspace.Name,
            IsArchived = workspace.IsArchived,
            ArchivedTime = workspace.ArchivedTime,
            ArchivedBy = workspace.ArchivedBy
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<DeletedWorkspaceDto> DeleteAsync(Guid id, DeleteWorkspaceDto input)
    {
        Check.NotNull(input, nameof(input));
        var userId = CurrentUser.GetId();

        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        var workspace = await _workspaceManager.DeleteAsync(id, userId, input.ConfirmationName);
        await uow.CompleteAsync();

        var deletionTime = workspace.DeletionTime ?? Clock.Now;
        var graceDays = LazyServiceProvider.LazyGetRequiredService<IOptions<DataErasureOptions>>().Value.WorkspaceGraceDays;
        return new DeletedWorkspaceDto
        {
            Id = workspace.Id,
            DeletionTime = deletionTime,
            PurgeAfter = deletionTime.AddDays(Math.Max(0, graceDays))
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceCaptureRetentionDto> GetCaptureRetentionAsync(Guid id)
    {
        var userId = CurrentUser.GetId();
        var member = await _memberships.FindActiveAsync(id, userId);
        if (member == null)
        {
            throw new AbpAuthorizationException("Only an active member of the workspace can view its capture retention policy.");
        }

        var (isEnabled, days) = await _workspaceSettings.GetCaptureRetentionPolicyAsync(id);
        return new WorkspaceCaptureRetentionDto
        {
            IsEnabled = isEnabled,
            RetentionDays = days,
            IsEnforced = _captureRetentionOptions.Value.IsEnabled
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceCaptureRetentionDto> UpdateCaptureRetentionAsync(Guid id, WorkspaceCaptureRetentionDto input)
    {
        Check.NotNull(input, nameof(input));

        if (input.RetentionDays < WorkspaceCaptureRetentionDto.MinRetentionDays ||
            input.RetentionDays > WorkspaceCaptureRetentionDto.MaxRetentionDays)
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new($"Retention days must be between {WorkspaceCaptureRetentionDto.MinRetentionDays} and {WorkspaceCaptureRetentionDto.MaxRetentionDays}.",
                    new[] { nameof(WorkspaceCaptureRetentionDto.RetentionDays) })
            });
        }

        var userId = CurrentUser.GetId();
        var member = await _memberships.FindActiveAsync(id, userId);
        if (member == null || !member.IsOwner)
        {
            throw new AbpAuthorizationException("Only an active owner of the workspace can change its capture retention policy.");
        }

        await _workspaceSettings.SetCaptureRetentionPolicyAsync(id, input.IsEnabled, input.RetentionDays);

        // Invalidate/reset continuation cursor when retention policy changes
        CaptureRetentionWorker.ResetCursor(id);

        return new WorkspaceCaptureRetentionDto
        {
            IsEnabled = input.IsEnabled,
            RetentionDays = input.RetentionDays,
            IsEnforced = _captureRetentionOptions.Value.IsEnabled
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceActivityRetentionDto> GetActivityRetentionAsync(Guid id)
    {
        if (await _memberships.FindActiveAsync(id, CurrentUser.GetId()) == null)
        {
            throw new AbpAuthorizationException("Only an active member of the workspace can view its activity retention policy.");
        }

        var (isEnabled, days) = await _workspaceSettings.GetActivityRetentionPolicyAsync(id);
        return new WorkspaceActivityRetentionDto
        {
            IsEnabled = isEnabled,
            RetentionDays = days,
            IsEnforced = _activityRetentionOptions.Value.IsEnabled
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<WorkspaceActivityRetentionDto> UpdateActivityRetentionAsync(Guid id, WorkspaceActivityRetentionDto input)
    {
        Check.NotNull(input, nameof(input));

        if (input.RetentionDays < ActivityRetentionLimits.MinDays || input.RetentionDays > ActivityRetentionLimits.MaxDays)
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new($"Retention days must be between {ActivityRetentionLimits.MinDays} and {ActivityRetentionLimits.MaxDays}.",
                    new[] { nameof(WorkspaceActivityRetentionDto.RetentionDays) })
            });
        }

        var member = await _memberships.FindActiveAsync(id, CurrentUser.GetId());
        if (member is not { IsOwner: true })
        {
            throw new AbpAuthorizationException("Only an active owner of the workspace can change its activity retention policy.");
        }

        await _workspaceSettings.SetActivityRetentionPolicyAsync(id, input.IsEnabled, input.RetentionDays);
        ActivityRetentionWorker.ResetCursor(id);

        return new WorkspaceActivityRetentionDto
        {
            IsEnabled = input.IsEnabled,
            RetentionDays = input.RetentionDays,
            IsEnforced = _activityRetentionOptions.Value.IsEnabled
        };
    }

    [AllowAnonymous]
    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<InvitationInfoDto> GetInvitationAsync(string token)
    {
        var invitation = await FindInvitationAsync(token)
                         ?? throw new BusinessException(TrackerDomainErrorCodes.InvitationNotFound);
        var workspace = await _workspaceRepository.GetAsync(invitation.WorkspaceId);

        return new InvitationInfoDto
        {
            WorkspaceName = workspace.Name,
            Email = invitation.Email,
            Role = invitation.Role,
            Expired = !invitation.IsUsable(Clock.Now)
        };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<AcceptInvitationResultDto> AcceptInvitationAsync(string token)
    {
        var invitation = await GetUsableInvitationAsync(token);
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        if (!invitation.IsFor(user.Email))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        using (CurrentWorkspace.Change(invitation.WorkspaceId))
        {
            await using var seatLock = await _seatCounter.LockAsync();
            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            var workspace = await AcceptAsync(invitation, user.Id);
            var role = (await _memberships.FindActiveAsync(workspace.Id, user.Id))!.Role;
            await uow.CompleteAsync();

            return new AcceptInvitationResultDto { WorkspaceId = workspace.Id, WorkspaceName = workspace.Name, Role = role };
        }
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task DeclineInvitationAsync(string token)
    {
        var invitation = await GetUsableInvitationAsync(token);
        var userId = CurrentUser.Id ?? throw new AbpAuthorizationException();
        var user = await _userManager.GetByIdAsync(userId);
        if (!invitation.IsFor(user.Email))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        await _invitationRepository.DeleteAsync(invitation);
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<MyInvitationsDto> GetMyInvitationsAsync()
    {
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        if (!IsVerifiedEmail(user))
        {
            return new MyInvitationsDto { EmailConfirmed = false };
        }

        var normalizedEmail = WorkspaceInvitation.NormalizeEmail(user.Email);
        var now = Clock.Now;
        var invitations = await _invitationRepository.GetQueryableAsync();
        var workspaces = await _workspaceRepository.GetQueryableAsync();
        var rows = await AsyncExecuter.ToListAsync(
            from i in invitations
            join w in workspaces on i.WorkspaceId equals w.Id
            where i.NormalizedEmail == normalizedEmail && i.AcceptedAt == null && i.ExpiresAt > now && !w.IsArchived
            orderby i.CreationTime descending
            select new MyInvitationDto
            {
                Id = i.Id,
                WorkspaceId = w.Id,
                WorkspaceName = w.Name,
                Role = i.Role,
                ExpiresAt = i.ExpiresAt,
                CreationTime = i.CreationTime
            });

        return new MyInvitationsDto { EmailConfirmed = true, Items = rows };
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task<AcceptInvitationResultDto> AcceptMyInvitationAsync(Guid id)
    {
        var (invitation, user) = await GetMyUsableInvitationAsync(id);

        using (CurrentWorkspace.Change(invitation.WorkspaceId))
        {
            await using var seatLock = await _seatCounter.LockAsync();
            using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);
            var workspace = await AcceptAsync(invitation, user.Id);
            var role = (await _memberships.FindActiveAsync(workspace.Id, user.Id))!.Role;
            await uow.CompleteAsync();

            return new AcceptInvitationResultDto { WorkspaceId = workspace.Id, WorkspaceName = workspace.Name, Role = role };
        }
    }

    [WorkspaceNotRequired]
    [RemoteService(IsEnabled = false)]
    public async Task DeclineMyInvitationAsync(Guid id)
    {
        var (invitation, _) = await GetMyUsableInvitationAsync(id);
        await _invitationRepository.DeleteAsync(invitation);
    }

    /// <summary>An invitation the caller may answer without its token: it is usable and addressed to the caller's
    /// verified email. Any other invitation is reported as not found, so ids reveal nothing.</summary>
    private async Task<(WorkspaceInvitation Invitation, IdentityUser User)> GetMyUsableInvitationAsync(Guid id)
    {
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        if (!IsVerifiedEmail(user))
        {
            throw new BusinessException(TrackerDomainErrorCodes.EmailNotConfirmed);
        }

        var invitation = await _invitationRepository.FindAsync(id);
        if (invitation == null || !invitation.IsFor(user.Email))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationNotFound);
        }

        if (!invitation.IsUsable(Clock.Now))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationExpired);
        }

        return (invitation, user);
    }

    /// <summary>Only a verified email may stand in for an invitation's token: anyone can sign up with any address.</summary>
    private static bool IsVerifiedEmail(IdentityUser user) => user.EmailConfirmed && !user.Email.IsNullOrWhiteSpace();

    /// <summary>Joins the invitation's workspace (the current one) and uses the invitation up.</summary>
    private async Task<Workspace> AcceptAsync(WorkspaceInvitation invitation, Guid userId)
    {
        var workspace = await _workspaceRepository.GetAsync(invitation.WorkspaceId);
        if (workspace.IsArchived)
        {
            throw new BusinessException(TrackerDomainErrorCodes.WorkspaceArchived);
        }

        var existing = await _memberships.FindActiveAsync(workspace.Id, userId);
        var projectId = invitation.ProjectId;
        if (projectId.HasValue)
        {
            var projectRepository = LazyServiceProvider.LazyGetRequiredService<IRepository<Project, Guid>>();
            if (await projectRepository.FindAsync(projectId.Value) == null)
            {
                projectId = null; // The project was deleted since: the invitation still opens the workspace.
            }
            else if (existing != null && await LazyServiceProvider.LazyGetRequiredService<IRepository<ProjectMember, Guid>>()
                         .AnyAsync(m => m.ProjectId == projectId && m.UserId == userId))
            {
                projectId = null;
            }
        }

        await _joiner.JoinAsync(userId, invitation.Role, projectId, invitation.ProjectRole, invitation.HourlyRate ?? 0m);
        invitation.MarkAccepted(userId, Clock.Now);
        await _invitationRepository.UpdateAsync(invitation, autoSave: true);
        return workspace;
    }

    private Task<WorkspaceInvitation?> FindInvitationAsync(string token)
    {
        if (token.IsNullOrWhiteSpace())
        {
            return Task.FromResult<WorkspaceInvitation?>(null);
        }

        var hash = WorkspaceInvitation.HashToken(token);
        return _invitationRepository.FirstOrDefaultAsync(i => i.TokenHash == hash);
    }

    private async Task<WorkspaceInvitation> GetUsableInvitationAsync(string token)
    {
        var invitation = await FindInvitationAsync(token)
                         ?? throw new BusinessException(TrackerDomainErrorCodes.InvitationNotFound);
        if (!invitation.IsUsable(Clock.Now))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvitationExpired);
        }

        return invitation;
    }

    public async Task<WorkspaceSettingsDto> GetSettingsAsync()
    {
        CurrentWorkspace.GetId();
        return new WorkspaceSettingsDto
        {
            DefaultTimeZone = await _workspaceSettings.GetDefaultTimeZoneAsync(),
            NewProjectDefaults = ToDto(await _workspaceSettings.GetNewProjectDefaultsAsync()),
            MemberCaptureDeleteMinutes = await _workspaceSettings.GetMemberCaptureDeleteMinutesAsync()
        };
    }

    public async Task<WorkspaceSettingsDto> UpdateSettingsAsync(WorkspaceSettingsDto input)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var member = await _memberships.FindActiveAsync(workspaceId, CurrentUser.GetId());
        if (member is not { IsOwner: true })
        {
            throw new AbpAuthorizationException("Only the workspace owner can change the workspace settings.");
        }

        var timeZone = NormalizeTimeZone(input.DefaultTimeZone, nameof(WorkspaceSettingsDto.DefaultTimeZone));
        var defaults = ToDomain(input.NewProjectDefaults);

        await _workspaceSettings.SetDefaultTimeZoneAsync(timeZone);
        await _workspaceSettings.SetNewProjectDefaultsAsync(defaults);
        if (input.MemberCaptureDeleteMinutes.HasValue)
        {
            await _workspaceSettings.SetMemberCaptureDeleteMinutesAsync(input.MemberCaptureDeleteMinutes.Value);
        }

        return new WorkspaceSettingsDto
        {
            DefaultTimeZone = timeZone,
            NewProjectDefaults = ToDto(defaults),
            MemberCaptureDeleteMinutes = await _workspaceSettings.GetMemberCaptureDeleteMinutesAsync()
        };
    }

    [WorkspaceNotRequired]
    public async Task<MyPreferencesDto> GetMyPreferencesAsync()
    {
        return new MyPreferencesDto
        {
            TimeZone = await _workspaceSettings.GetUserTimeZoneAsync()
        };
    }

    [WorkspaceNotRequired]
    public async Task<MyPreferencesDto> UpdateMyPreferencesAsync(MyPreferencesDto input)
    {
        var timeZone = NormalizeTimeZone(input.TimeZone, nameof(MyPreferencesDto.TimeZone));
        await _workspaceSettings.SetUserTimeZoneAsync(timeZone);
        return new MyPreferencesDto { TimeZone = timeZone };
    }

    /// <summary>The plan chosen at sign-up: by id, else by name, else the default (Free). Self-service accepts free
    /// plans and plans with a trial; a paid plan without one would start active with nothing paid. While billing is off
    /// the choice is ignored: every new workspace gets the default plan (which then limits nothing).</summary>
    private async Task<Plan> GetSignUpPlanAsync(Guid? planId, string? planNameInput)
    {
        if (!IsBillingEnabled)
        {
            planId = null;
            planNameInput = null;
        }

        Plan? plan;
        string member;
        if (planId.HasValue)
        {
            member = nameof(RegisterWorkspaceDto.PlanId);
            plan = await _planRepository.FindAsync(planId.Value);
            if (plan == null)
            {
                throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                    .WithData("planId", planId.Value);
            }
        }
        else
        {
            member = nameof(RegisterWorkspaceDto.PlanName);
            var planName = planNameInput.IsNullOrWhiteSpace() ? DefaultPlanName : planNameInput!.Trim();
            plan = await _planRepository.FirstOrDefaultAsync(p => p.Name == planName);
            if (plan == null)
            {
                throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                    .WithData("planName", planName);
            }
        }

        if (!plan.IsFree() && plan.TrialDays <= 0)
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new($"The {plan.Name} plan can't be chosen at sign-up. Start on a free plan or a plan with a free trial.",
                    new[] { member })
            });
        }

        return plan;
    }

    private bool IsBillingEnabled => LazyServiceProvider.LazyGetRequiredService<IOptions<BillingOptions>>().Value.Enabled;

    /// <summary>Emails a new account the link that confirms its address (best effort).</summary>
    private Task<bool> SendEmailConfirmationAsync(IdentityUser user)
    {
        return LazyServiceProvider.LazyGetRequiredService<AccountLinkEmails>().SendEmailConfirmationAsync(user);
    }

    private static NewProjectDefaultsDto ToDto(NewProjectDefaults defaults)
    {
        return new NewProjectDefaultsDto
        {
            IntervalMinutes = defaults.IntervalMinutes,
            AllowScreenshot = defaults.AllowScreenshot,
            AllowWebcam = defaults.AllowWebcam,
            AllowKeyboard = defaults.AllowKeyboard,
            AllowMouse = defaults.AllowMouse,
            AllowActiveWindow = defaults.AllowActiveWindow,
            AllowRunningPrograms = defaults.AllowRunningPrograms,
            BlurScreenshots = defaults.BlurScreenshots,
            IdleThresholdMinutes = defaults.IdleThresholdMinutes
        };
    }

    private static NewProjectDefaults ToDomain(NewProjectDefaultsDto defaults)
    {
        return new NewProjectDefaults
        {
            IntervalMinutes = defaults.IntervalMinutes,
            AllowScreenshot = defaults.AllowScreenshot,
            AllowWebcam = defaults.AllowWebcam,
            AllowKeyboard = defaults.AllowKeyboard,
            AllowMouse = defaults.AllowMouse,
            AllowActiveWindow = defaults.AllowActiveWindow,
            AllowRunningPrograms = defaults.AllowRunningPrograms,
            BlurScreenshots = defaults.BlurScreenshots,
            IdleThresholdMinutes = defaults.IdleThresholdMinutes
        };
    }

    /// <summary>Null for a blank id (unset); otherwise the IANA form, or a validation error for an unknown zone.</summary>
    private static string? NormalizeTimeZone(string? timeZoneId, string member)
    {
        if (timeZoneId.IsNullOrWhiteSpace())
        {
            return null;
        }

        if (TimeZoneIds.TryNormalize(timeZoneId, out var ianaId))
        {
            return ianaId;
        }

        throw new AbpValidationException(new List<ValidationResult>
        {
            new($"'{timeZoneId}' is not a known time zone. Use an IANA id such as \"Asia/Dhaka\".", new[] { member })
        });
    }
}
