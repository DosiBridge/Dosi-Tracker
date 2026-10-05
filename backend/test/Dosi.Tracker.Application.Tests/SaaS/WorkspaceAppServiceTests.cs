using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Settings;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.AuditLogging;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Security.Claims;
using Volo.Abp.TenantManagement;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class WorkspaceAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IWorkspaceAppService _workspaceAppService;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly ITenantRepository _tenantRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;
    private readonly ICurrentPrincipalAccessor _principalAccessor;

    protected WorkspaceAppServiceTests()
    {
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private static RegisterWorkspaceDto SignUp(string name, string? planName = null, string password = "1q2w3E*owner")
    {
        return new RegisterWorkspaceDto
        {
            Name = name,
            AdminEmail = $"owner@{name}.test",
            AdminPassword = password,
            PlanName = planName
        };
    }

    private Task<bool> WorkspaceExistsAsync(string name)
    {
        return WithUnitOfWorkAsync(async () => await _tenantRepository.FindByNameAsync(name.ToUpperInvariant()) != null);
    }

    /// <summary>Acts as a member of the workspace, optionally as its owner (Identity "admin" role).</summary>
    private IDisposable ActAs(Guid userId, bool owner)
    {
        var claims = new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) };
        if (owner)
        {
            claims.Add(new Claim(AbpClaimTypes.Role, TrackerRoles.Admin));
        }

        return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "Test")));
    }

    private static WorkspaceSettingsDto Settings(string? timeZone, int intervalMinutes = 20, int idleThresholdMinutes = 10)
    {
        return new WorkspaceSettingsDto
        {
            DefaultTimeZone = timeZone,
            NewProjectDefaults = new NewProjectDefaultsDto
            {
                IntervalMinutes = intervalMinutes,
                AllowScreenshot = false,
                AllowWebcam = true,
                AllowKeyboard = true,
                AllowMouse = false,
                AllowActiveWindow = false,
                AllowRunningPrograms = true,
                BlurScreenshots = true,
                IdleThresholdMinutes = idleThresholdMinutes
            }
        };
    }

    // ---- Plans ----

    [Fact]
    public async Task GetAvailablePlans_Should_Return_The_Seeded_Default_Plans()
    {
        var plans = await _workspaceAppService.GetAvailablePlansAsync();

        plans.Items.Count.ShouldBeGreaterThanOrEqualTo(3);
        plans.Items.ShouldContain(p => p.Name == "Free" && p.PricePerUser == 0m && p.MaxSeats == 3);
        plans.Items.ShouldContain(p => p.Name == "Starter" && p.PricePerUser == 6m && p.TrialDays == 14);
        plans.Items.ShouldContain(p => p.Name == "Business" && p.PricePerUser == 12m);
    }

    [Fact]
    public void GetAvailablePlans_Should_Be_Anonymous_For_The_Sign_Up_Page()
    {
        typeof(WorkspaceAppService)
            .GetMethod(nameof(WorkspaceAppService.GetAvailablePlansAsync))!
            .GetCustomAttribute<AllowAnonymousAttribute>()
            .ShouldNotBeNull();
    }

    [Fact]
    public async Task GetAvailablePlans_Should_List_The_Plans_As_Stored_Cheapest_First_Without_Editor_Ids()
    {
        // Inserted as the (fake) signed-in platform administrator, so the plan records who created it.
        await WithUnitOfWorkAsync(() => _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Pro", 9m, 50, 7)));
        (await WithUnitOfWorkAsync(() => _planRepository.FirstAsync(p => p.Name == "Pro"))).CreatorId.ShouldNotBeNull();

        var plans = (await _workspaceAppService.GetAvailablePlansAsync()).Items;

        plans.Select(p => p.PricePerUser).ShouldBeInOrder(SortDirection.Ascending);
        plans.ShouldContain(p => p.Name == "Pro" && p.PricePerUser == 9m && p.TrialDays == 7);
        plans.ShouldAllBe(p => p.CreatorId == null && p.LastModifierId == null && p.DeleterId == null);
    }

    // ---- Subscription ----

    [Fact]
    public async Task GetCurrentSubscription_Should_Return_Null_When_None_Exists()
    {
        var subscription = await _workspaceAppService.GetCurrentSubscriptionAsync();

        subscription.ShouldBeNull();
    }

    [Fact]
    public async Task GetCurrentSubscription_Should_Return_The_Tenant_Subscription()
    {
        var planId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            // Test host runs without a tenant, so TenantId == null matches CurrentTenant.Id.
            await _subscriptionRepository.InsertAsync(
                new Subscription(_guidGenerator.Create(), null, planId));
        });

        var subscription = await _workspaceAppService.GetCurrentSubscriptionAsync();

        subscription.ShouldNotBeNull();
        subscription.PlanId.ShouldBe(planId);
        subscription.Status.ShouldBe("trialing");
    }

    // ---- Sign-up ----

    [Fact]
    public async Task Register_Should_Create_Tenant_Admin_And_Trialing_Subscription()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "acme",
            AdminEmail = "owner@acme.test",
            AdminPassword = "1q2w3E*acme",
            PlanName = "Starter"
        });

        result.TenantId.ShouldNotBe(Guid.Empty);
        result.Name.ShouldBe("acme");

        await WithUnitOfWorkAsync(async () =>
        {
            var tenant = await _tenantRepository.FindAsync(result.TenantId);
            tenant.ShouldNotBeNull();
            tenant.Name.ShouldBe("acme");
        });

        // The tenant got its admin user and a trialing Starter subscription.
        using (_currentTenant.Change(result.TenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var userRepository = GetRequiredService<IIdentityUserRepository>();
                var users = await userRepository.GetListAsync();
                users.ShouldContain(u => u.Email == "owner@acme.test");

                var subscription = await _subscriptionRepository.FirstOrDefaultAsync(
                    s => s.TenantId == result.TenantId);
                subscription.ShouldNotBeNull();
                subscription.Status.ShouldBe("trialing");
                subscription.TrialEndsAt.ShouldNotBeNull();
            });
        }
    }

    [Fact]
    public async Task Register_Should_Activate_Immediately_On_The_Free_Plan()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "freeco",
            AdminEmail = "owner@freeco.test",
            AdminPassword = "1q2w3E*freeco"
        });

        using (_currentTenant.Change(result.TenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                var subscription = await _subscriptionRepository.FirstOrDefaultAsync(
                    s => s.TenantId == result.TenantId);
                subscription.ShouldNotBeNull();
                subscription.Status.ShouldBe("active"); // nothing to trial on Free
                subscription.TrialEndsAt.ShouldBeNull();
            });
        }
    }

    [Fact]
    public async Task Register_Should_Reject_Unknown_Plans()
    {
        var exception = await Should.ThrowAsync<BusinessException>(
            () => _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
            {
                Name = "nope",
                AdminEmail = "owner@nope.test",
                AdminPassword = "1q2w3E*nope",
                PlanName = "Platinum"
            }));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
    }

    [Fact]
    public async Task Register_Should_Accept_A_Plan_Id_From_The_Plan_List()
    {
        var business = (await _workspaceAppService.GetAvailablePlansAsync()).Items.Single(p => p.Name == "Business");

        var input = SignUp("byid", planName: "Starter"); // the id wins over the name
        input.PlanId = business.Id;
        var result = await _workspaceAppService.RegisterAsync(input);

        using (_currentTenant.Change(result.TenantId))
        {
            var subscription = await WithUnitOfWorkAsync(() => _subscriptionRepository.FirstAsync(s => s.TenantId == result.TenantId));
            subscription.PlanId.ShouldBe(business.Id);
            subscription.Status.ShouldBe(Subscription.TrialingStatus);
        }
    }

    [Fact]
    public async Task Register_Should_Reject_An_Unknown_Plan_Id()
    {
        var input = SignUp("ghostplan");
        input.PlanId = Guid.NewGuid();

        var exception = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.RegisterAsync(input));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
        (await WorkspaceExistsAsync("ghostplan")).ShouldBeFalse();
    }

    [Fact]
    public async Task Register_Should_Refuse_A_Paid_Plan_Without_A_Trial()
    {
        // It would start active (and be invoiced) without anyone having agreed to pay.
        await WithUnitOfWorkAsync(() => _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Enterprise", 25m, 500, 0)));

        var exception = await Should.ThrowAsync<AbpValidationException>(
            () => _workspaceAppService.RegisterAsync(SignUp("bigco", planName: "Enterprise")));

        exception.ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(RegisterWorkspaceDto.PlanName)));
        (await WorkspaceExistsAsync("bigco")).ShouldBeFalse();
    }

    [Theory]
    [InlineData("a")]
    [InlineData("12345")]
    public async Task Register_Should_Enforce_The_Password_Policy(string password)
    {
        var exception = await Should.ThrowAsync<AbpValidationException>(
            () => _workspaceAppService.RegisterAsync(SignUp("weakco", password: password)));

        exception.ValidationErrors.ShouldNotBeEmpty();
        exception.ValidationErrors.ShouldAllBe(e => e.MemberNames.Contains(nameof(RegisterWorkspaceDto.AdminPassword)));
        (await WorkspaceExistsAsync("weakco")).ShouldBeFalse(); // checked before anything is created
    }

    [Fact]
    public async Task Register_Should_Keep_The_Owner_Password_Out_Of_The_Audit_Log()
    {
        const string password = "Sup3r-Secret-Owner!";
        await _workspaceAppService.RegisterAsync(SignUp("audited", password: password));

        var logs = await WithUnitOfWorkAsync(() => GetRequiredService<IAuditLogRepository>().GetListAsync(includeDetails: true));
        var action = logs.SelectMany(l => l.Actions)
            .Where(a => a.MethodName == nameof(IWorkspaceAppService.RegisterAsync))
            .ShouldHaveSingleItem();

        action.Parameters.ShouldContain("audited"); // the sign-up itself stays traceable
        action.Parameters.ShouldContain("owner@audited.test");
        action.Parameters.ShouldNotContain(password);
        action.Parameters.ShouldNotContain("adminPassword", Case.Insensitive);
    }

    [Fact]
    public async Task Register_Should_Reject_A_Workspace_Name_In_Use()
    {
        await _workspaceAppService.RegisterAsync(SignUp("twice"));

        var exception = await Should.ThrowAsync<BusinessException>(
            () => _workspaceAppService.RegisterAsync(SignUp("TWICE")));

        exception.Code.ShouldBe(WorkspaceAppService.DuplicateTenantNameErrorCode);
    }

    [Fact]
    public async Task Register_Should_Trim_The_Workspace_Name()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "  spaced  ",
            AdminEmail = "owner@spaced.test",
            AdminPassword = "1q2w3E*spaced"
        });

        result.Name.ShouldBe("spaced");
        (await WorkspaceExistsAsync("spaced")).ShouldBeTrue();
    }

    // ---- Workspace settings ----

    [Fact]
    public async Task Settings_Should_Start_From_The_Platform_Defaults_And_Be_Readable_By_Any_Member()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid(), owner: false))
        {
            var settings = await _workspaceAppService.GetSettingsAsync();

            settings.DefaultTimeZone.ShouldBeNull();
            settings.NewProjectDefaults.ShouldBeEquivalentTo(new NewProjectDefaultsDto());
        }
    }

    [Fact]
    public async Task The_Owner_Should_Change_The_Settings_Of_Their_Workspace_Only()
    {
        var tenantId = Guid.NewGuid();
        using (_currentTenant.Change(tenantId))
        using (ActAs(Guid.NewGuid(), owner: true))
        {
            var updated = await _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka"));

            updated.ShouldBeEquivalentTo(Settings("Asia/Dhaka"));
            (await _workspaceAppService.GetSettingsAsync()).ShouldBeEquivalentTo(Settings("Asia/Dhaka"));
        }

        using (_currentTenant.Change(Guid.NewGuid()))
        {
            (await _workspaceAppService.GetSettingsAsync()).DefaultTimeZone.ShouldBeNull();
        }
    }

    [Fact]
    public async Task A_Windows_Time_Zone_Should_Be_Stored_As_Its_Iana_Id_And_A_Blank_One_Should_Unset_It()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid(), owner: true))
        {
            (await _workspaceAppService.UpdateSettingsAsync(Settings("Bangladesh Standard Time"))).DefaultTimeZone.ShouldBe("Asia/Dhaka");
            (await _workspaceAppService.UpdateSettingsAsync(Settings("  "))).DefaultTimeZone.ShouldBeNull();
        }
    }

    [Fact]
    public async Task Only_The_Workspace_Owner_Should_Change_The_Settings()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid(), owner: false))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka")));
            (await _workspaceAppService.GetSettingsAsync()).DefaultTimeZone.ShouldBeNull();
        }
    }

    [Fact]
    public async Task The_Host_Should_Have_No_Workspace_Settings_To_Change()
    {
        using (ActAs(Guid.NewGuid(), owner: true))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka")));
        }
    }

    [Fact]
    public async Task An_Unknown_Time_Zone_Should_Be_A_Validation_Error()
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid(), owner: true))
        {
            var exception = await Should.ThrowAsync<AbpValidationException>(
                () => _workspaceAppService.UpdateSettingsAsync(Settings("Mars/Olympus_Mons")));

            exception.ValidationErrors.ShouldHaveSingleItem().MemberNames.ShouldContain(nameof(WorkspaceSettingsDto.DefaultTimeZone));
            (await _workspaceAppService.GetSettingsAsync()).ShouldBeEquivalentTo(new WorkspaceSettingsDto());
        }
    }

    [Theory]
    [InlineData(4, 5)]
    [InlineData(61, 5)]
    [InlineData(10, 61)]
    [InlineData(10, -1)]
    public async Task Out_Of_Range_New_Project_Defaults_Should_Be_A_Validation_Error(int intervalMinutes, int idleThresholdMinutes)
    {
        using (_currentTenant.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid(), owner: true))
        {
            await Should.ThrowAsync<AbpValidationException>(() => _workspaceAppService.UpdateSettingsAsync(
                Settings("Asia/Dhaka", intervalMinutes, idleThresholdMinutes)));
        }
    }

    // ---- My preferences ----

    [Fact]
    public async Task My_Time_Zone_Should_Be_Mine_Alone_And_Can_Be_Cleared()
    {
        var tenantId = Guid.NewGuid();
        var alice = Guid.NewGuid();
        using (_currentTenant.Change(tenantId))
        {
            using (ActAs(alice, owner: false))
            {
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
                (await _workspaceAppService.UpdateMyPreferencesAsync(new MyPreferencesDto { TimeZone = "Europe/London" }))
                    .TimeZone.ShouldBe("Europe/London");
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBe("Europe/London");
            }

            using (ActAs(Guid.NewGuid(), owner: false))
            {
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
            }

            using (ActAs(alice, owner: false))
            {
                (await _workspaceAppService.UpdateMyPreferencesAsync(new MyPreferencesDto { TimeZone = null })).TimeZone.ShouldBeNull();
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
            }
        }
    }

    [Fact]
    public async Task My_Time_Zone_Should_Be_Validated()
    {
        var exception = await Should.ThrowAsync<AbpValidationException>(
            () => _workspaceAppService.UpdateMyPreferencesAsync(new MyPreferencesDto { TimeZone = "Not/AZone" }));

        exception.ValidationErrors.ShouldHaveSingleItem().MemberNames.ShouldContain(nameof(MyPreferencesDto.TimeZone));
        (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
    }
}
