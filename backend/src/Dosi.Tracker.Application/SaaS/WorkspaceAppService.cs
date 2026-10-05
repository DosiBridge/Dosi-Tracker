using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Settings;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Authorization;
using Volo.Abp.Data;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Volo.Abp.Uow;
using Volo.Abp.Validation;
using IdentityOptions = Microsoft.AspNetCore.Identity.IdentityOptions;

namespace Dosi.Tracker.SaaS;

[Authorize]
public class WorkspaceAppService : TrackerAppService, IWorkspaceAppService
{
    public const string DefaultPlanName = Plan.FreePlanName;

    /// <summary>ABP's error code for a workspace name already in use (raised by its tenant validator).</summary>
    public const string DuplicateTenantNameErrorCode = "Volo.Abp.TenantManagement:DuplicateTenantName";

    /// <summary>Prefix of the lock that serializes sign-ups of the same workspace name.</summary>
    public const string RegistrationLockPrefix = "Tracker:WorkspaceRegistration:";

    /// <summary>User name ABP's identity seeder gives a new workspace's owner.</summary>
    private const string OwnerUserName = "admin";

    /// <summary>How long a sign-up waits for another sign-up of the same name to finish.</summary>
    private static readonly TimeSpan RegistrationLockTimeout = TimeSpan.FromSeconds(30);

    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;
    private readonly ITenantNormalizer _tenantNormalizer;
    private readonly IDataSeeder _dataSeeder;
    private readonly IdentityUserManager _userManager;
    private readonly IOptions<IdentityOptions> _identityOptions;
    private readonly IAbpDistributedLock _distributedLock;
    private readonly SubscriptionManager _subscriptionManager;
    private readonly WorkspaceSettingsManager _workspaceSettings;

    public WorkspaceAppService(
        IRepository<Subscription, Guid> subscriptionRepository,
        IRepository<Plan, Guid> planRepository,
        ITenantManager tenantManager,
        ITenantRepository tenantRepository,
        ITenantNormalizer tenantNormalizer,
        IDataSeeder dataSeeder,
        IdentityUserManager userManager,
        IOptions<IdentityOptions> identityOptions,
        IAbpDistributedLock distributedLock,
        SubscriptionManager subscriptionManager,
        WorkspaceSettingsManager workspaceSettings)
    {
        _subscriptionRepository = subscriptionRepository;
        _planRepository = planRepository;
        _tenantManager = tenantManager;
        _tenantRepository = tenantRepository;
        _tenantNormalizer = tenantNormalizer;
        _dataSeeder = dataSeeder;
        _userManager = userManager;
        _identityOptions = identityOptions;
        _distributedLock = distributedLock;
        _subscriptionManager = subscriptionManager;
        _workspaceSettings = workspaceSettings;
    }

    public async Task<SubscriptionDto?> GetCurrentSubscriptionAsync()
    {
        var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.TenantId == CurrentTenant.Id);
        if (subscription == null) return null;

        return ObjectMapper.Map<Subscription, SubscriptionDto>(subscription);
    }

    /// <summary>Anonymous: the sign-up page offers the plans as the host currently defines them.</summary>
    [AllowAnonymous]
    public async Task<ListResultDto<PlanDto>> GetAvailablePlansAsync()
    {
        var plans = (await _planRepository.GetListAsync())
            .OrderBy(p => p.PricePerUser)
            .ThenBy(p => p.Name)
            .ToList();

        var dtos = ObjectMapper.Map<List<Plan>, List<PlanDto>>(plans);
        foreach (var dto in dtos)
        {
            // Anonymous callers see the offer, not which platform administrator edited it.
            dto.CreatorId = null;
            dto.LastModifierId = null;
            dto.DeleterId = null;
        }

        return new ListResultDto<PlanDto>(dtos);
    }

    [AllowAnonymous]
    public async Task<WorkspaceRegistrationResultDto> RegisterAsync(RegisterWorkspaceDto input)
    {
        var plan = await GetSignUpPlanAsync(input);
        await CheckOwnerPasswordAsync(input);

        var name = input.Name.Trim();
        var normalizedName = _tenantNormalizer.NormalizeName(name) ?? name;

        // Sign-ups of the same name run one at a time (across replicas when a distributed lock provider is
        // configured), and the lock is held until the new workspace is committed, so the name check can't be raced.
        await using var nameLock = await _distributedLock.TryAcquireAsync(RegistrationLockPrefix + normalizedName, RegistrationLockTimeout);
        if (nameLock == null)
        {
            throw DuplicateTenantName(normalizedName); // Another sign-up of this name is still running.
        }

        using var uow = UnitOfWorkManager.Begin(requiresNew: true, isTransactional: true);

        var tenant = await CreateTenantAsync(name, normalizedName);
        using (CurrentTenant.Change(tenant.Id, tenant.Name))
        {
            // Seeds the tenant's admin user (Identity), admin role and its permissions.
            await _dataSeeder.SeedAsync(new DataSeedContext(tenant.Id)
                .WithProperty(IdentityDataSeedContributor.AdminEmailPropertyName, input.AdminEmail)
                .WithProperty(IdentityDataSeedContributor.AdminPasswordPropertyName, input.AdminPassword));

            await _subscriptionRepository.InsertAsync(_subscriptionManager.CreateForNewWorkspace(tenant.Id, plan));
        }

        await uow.CompleteAsync();

        return new WorkspaceRegistrationResultDto
        {
            TenantId = tenant.Id,
            Name = tenant.Name
        };
    }

    public async Task<WorkspaceSettingsDto> GetSettingsAsync()
    {
        return new WorkspaceSettingsDto
        {
            DefaultTimeZone = await _workspaceSettings.GetDefaultTimeZoneAsync(),
            NewProjectDefaults = ToDto(await _workspaceSettings.GetNewProjectDefaultsAsync())
        };
    }

    public async Task<WorkspaceSettingsDto> UpdateSettingsAsync(WorkspaceSettingsDto input)
    {
        if (CurrentTenant.Id == null)
        {
            throw new AbpAuthorizationException("Workspace settings exist inside a workspace only.");
        }

        if (!CurrentUser.IsInRole(TrackerRoles.Admin))
        {
            throw new AbpAuthorizationException("Only the workspace owner can change the workspace settings.");
        }

        var timeZone = NormalizeTimeZone(input.DefaultTimeZone, nameof(WorkspaceSettingsDto.DefaultTimeZone));
        var defaults = ToDomain(input.NewProjectDefaults);

        await _workspaceSettings.SetDefaultTimeZoneAsync(timeZone);
        await _workspaceSettings.SetNewProjectDefaultsAsync(defaults);

        // Answer with what was stored rather than reading it back: ABP's setting cache may still serve a value
        // removed in this unit of work (an unset time zone) until the unit of work completes.
        return new WorkspaceSettingsDto
        {
            DefaultTimeZone = timeZone,
            NewProjectDefaults = ToDto(defaults)
        };
    }

    public async Task<MyPreferencesDto> GetMyPreferencesAsync()
    {
        return new MyPreferencesDto
        {
            TimeZone = await _workspaceSettings.GetUserTimeZoneAsync()
        };
    }

    public async Task<MyPreferencesDto> UpdateMyPreferencesAsync(MyPreferencesDto input)
    {
        var timeZone = NormalizeTimeZone(input.TimeZone, nameof(MyPreferencesDto.TimeZone));
        await _workspaceSettings.SetUserTimeZoneAsync(timeZone);

        // What was stored (see UpdateSettingsAsync for why it is not read back).
        return new MyPreferencesDto { TimeZone = timeZone };
    }

    /// <summary>The plan chosen at sign-up: by id, else by name, else the default (Free). Self-service sign-up
    /// accepts free plans and plans with a trial; a paid plan without one would start active with nothing paid.</summary>
    private async Task<Plan> GetSignUpPlanAsync(RegisterWorkspaceDto input)
    {
        Plan? plan;
        string member;
        if (input.PlanId.HasValue)
        {
            member = nameof(RegisterWorkspaceDto.PlanId);
            plan = await _planRepository.FindAsync(input.PlanId.Value);
            if (plan == null)
            {
                throw new BusinessException(TrackerDomainErrorCodes.PlanNotFound)
                    .WithData("planId", input.PlanId.Value);
            }
        }
        else
        {
            member = nameof(RegisterWorkspaceDto.PlanName);
            var planName = input.PlanName.IsNullOrWhiteSpace() ? DefaultPlanName : input.PlanName!.Trim();
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

    /// <summary>The owner's password must meet the identity password policy (length, required characters, …):
    /// ABP's data seeder creates the owner account without checking it.</summary>
    private async Task CheckOwnerPasswordAsync(RegisterWorkspaceDto input)
    {
        await _identityOptions.SetAsync(); // load the policy from the settings

        var owner = new IdentityUser(GuidGenerator.Create(), OwnerUserName, input.AdminEmail);
        var errors = new List<ValidationResult>();
        foreach (var validator in _userManager.PasswordValidators)
        {
            var result = await validator.ValidateAsync(_userManager, owner, input.AdminPassword);
            errors.AddRange(result.Errors.Select(error =>
                new ValidationResult(error.Description, new[] { nameof(RegisterWorkspaceDto.AdminPassword) })));
        }

        if (errors.Count > 0)
        {
            throw new AbpValidationException(errors);
        }
    }

    /// <summary>Creates and saves the tenant. ABP's validator refuses a name already in use; a concurrent sign-up
    /// that commits the same name first (the unique index on the normalized name rejects our insert) gets the same
    /// error rather than a server error.</summary>
    private async Task<Tenant> CreateTenantAsync(string name, string normalizedName)
    {
        var tenant = await _tenantManager.CreateAsync(name);
        try
        {
            await _tenantRepository.InsertAsync(tenant, autoSave: true);
        }
        catch (Exception ex) when (ex is not BusinessException)
        {
            if (await IsTenantNameTakenAsync(normalizedName))
            {
                throw DuplicateTenantName(normalizedName);
            }

            throw;
        }

        return tenant;
    }

    private async Task<bool> IsTenantNameTakenAsync(string normalizedName)
    {
        // A fresh unit of work: the failed insert leaves the current one's database context (and transaction) unusable.
        using var uow = UnitOfWorkManager.Begin(requiresNew: true);
        var taken = await _tenantRepository.FindByNameAsync(normalizedName, includeDetails: false) != null;
        await uow.CompleteAsync();
        return taken;
    }

    private static BusinessException DuplicateTenantName(string normalizedName)
    {
        return new BusinessException(DuplicateTenantNameErrorCode).WithData("Name", normalizedName);
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
