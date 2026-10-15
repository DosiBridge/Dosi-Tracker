using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
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
using Volo.Abp.Security.Claims;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class WorkspaceAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IWorkspaceAppService _workspaceAppService;
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly ICurrentPrincipalAccessor _principalAccessor;

    protected WorkspaceAppServiceTests()
    {
        _workspaceAppService = GetRequiredService<IWorkspaceAppService>();
        _planRepository = GetRequiredService<IRepository<Plan, Guid>>();
        _subscriptionRepository = GetRequiredService<IRepository<Subscription, Guid>>();
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
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
            WorkspaceName = name,
            Email = $"owner@{name.Trim()}.test",
            Password = password,
            PlanName = planName,
            FirstName = "Olivia",
            Surname = "Owner"
        };
    }

    private Task<Workspace?> FindWorkspaceAsync(string name)
    {
        var normalized = Workspace.NormalizeName(name);
        return WithUnitOfWorkAsync(() => _workspaceRepository.FirstOrDefaultAsync(w => w.NormalizedName == normalized));
    }

    private Task<IdentityUser?> FindUserByEmailAsync(string email)
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync(email));
    }

    private IDisposable ActAs(Guid userId, string? email = null)
    {
        var claims = new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) };
        if (email != null)
        {
            claims.Add(new Claim(AbpClaimTypes.Email, email));
        }

        return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "Test")));
    }

    /// <summary>A new account (and, with a role, an active membership of the workspace).</summary>
    private async Task<Guid> CreateUserAsync(string email, string? role = null, Guid? workspaceId = null)
    {
        var userId = await WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(_guidGenerator.Create(), email, email);
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user, "1q2w3E*user")).Succeeded.ShouldBeTrue();
            return user.Id;
        });
        if (role != null)
        {
            await ServiceProvider.AddWorkspaceMemberAsync(userId, role, workspaceId);
        }

        return userId;
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
    public void Sign_Up_And_Its_Plans_Should_Be_Anonymous_And_Need_No_Workspace()
    {
        foreach (var method in new[] { nameof(WorkspaceAppService.GetAvailablePlansAsync), nameof(WorkspaceAppService.RegisterAsync) })
        {
            var info = typeof(WorkspaceAppService).GetMethod(method)!;
            info.GetCustomAttribute<AllowAnonymousAttribute>().ShouldNotBeNull();
            info.GetCustomAttribute<WorkspaceNotRequiredAttribute>().ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task GetAvailablePlans_Should_List_The_Plans_As_Stored_Cheapest_First_Without_Editor_Ids()
    {
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
        (await _workspaceAppService.GetCurrentSubscriptionAsync()).ShouldBeNull();
    }

    [Fact]
    public async Task GetCurrentSubscription_Should_Return_The_Workspace_Subscription()
    {
        var planId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(() => _subscriptionRepository.InsertAsync(
            new Subscription(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, planId)));

        var subscription = await _workspaceAppService.GetCurrentSubscriptionAsync();

        subscription.ShouldNotBeNull();
        subscription.PlanId.ShouldBe(planId);
        subscription.Status.ShouldBe("trialing");

        using (_currentWorkspace.Change(Guid.NewGuid()))
        {
            (await _workspaceAppService.GetCurrentSubscriptionAsync()).ShouldBeNull();
        }
    }

    [Fact]
    public async Task GetCurrentSubscription_Without_A_Workspace_Should_Require_One()
    {
        using (_currentWorkspace.Change(null))
        {
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.GetCurrentSubscriptionAsync()))
                .Code.ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);
        }
    }

    // ---- Sign-up ----

    [Fact]
    public async Task Register_Should_Create_The_Account_The_Workspace_Its_Owner_And_A_Trialing_Subscription()
    {
        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            WorkspaceName = "acme",
            Email = "owner@acme.test",
            Password = "1q2w3E*acme",
            FirstName = "Ada",
            Surname = "Lovelace",
            PlanName = "Starter"
        });

        result.WorkspaceId.ShouldNotBe(Guid.Empty);
        result.WorkspaceName.ShouldBe("acme");
        result.UserName.ShouldBe("owner@acme.test");

        var user = (await FindUserByEmailAsync("owner@acme.test")).ShouldNotBeNull();
        user.Name.ShouldBe("Ada");
        user.Surname.ShouldBe("Lovelace");
        user.TenantId.ShouldBeNull();
        (await ServiceProvider.FindWorkspaceMemberAsync(user.Id, result.WorkspaceId))!.Role.ShouldBe(WorkspaceRoles.Owner);

        using (_currentWorkspace.Change(result.WorkspaceId))
        {
            var subscription = await WithUnitOfWorkAsync(() => _subscriptionRepository.FirstAsync(s => s.WorkspaceId == result.WorkspaceId));
            subscription.Status.ShouldBe("trialing");
            subscription.TrialEndsAt.ShouldNotBeNull();
        }
    }

    [Fact]
    public async Task Register_Should_Activate_Immediately_On_The_Free_Plan()
    {
        var result = await _workspaceAppService.RegisterAsync(SignUp("freeco"));

        using (_currentWorkspace.Change(result.WorkspaceId))
        {
            var subscription = await WithUnitOfWorkAsync(() => _subscriptionRepository.FirstAsync(s => s.WorkspaceId == result.WorkspaceId));
            subscription.Status.ShouldBe("active");
            subscription.TrialEndsAt.ShouldBeNull();
        }
    }

    [Fact]
    public async Task Register_Should_Refuse_An_Email_That_Already_Has_An_Account()
    {
        await _workspaceAppService.RegisterAsync(SignUp("first"));

        var input = SignUp("second");
        input.Email = "OWNER@first.test";
        var exception = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.RegisterAsync(input));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.EmailAlreadyRegistered);
        (await FindWorkspaceAsync("second")).ShouldBeNull();
    }

    [Fact]
    public async Task Register_Without_A_Workspace_Name_Or_Invitation_Should_Be_A_Validation_Error()
    {
        var input = SignUp("unnamed");
        input.WorkspaceName = " ";

        var exception = await Should.ThrowAsync<AbpValidationException>(() => _workspaceAppService.RegisterAsync(input));

        exception.ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(RegisterWorkspaceDto.WorkspaceName)));
    }

    [Fact]
    public async Task Register_Should_Reject_Unknown_Plans()
    {
        var exception = await Should.ThrowAsync<BusinessException>(
            () => _workspaceAppService.RegisterAsync(SignUp("nope", planName: "Platinum")));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.PlanNotFound);
        (await FindUserByEmailAsync("owner@nope.test")).ShouldBeNull();
    }

    [Fact]
    public async Task Register_Should_Accept_A_Plan_Id_From_The_Plan_List()
    {
        var business = (await _workspaceAppService.GetAvailablePlansAsync()).Items.Single(p => p.Name == "Business");

        var input = SignUp("byid", planName: "Starter");
        input.PlanId = business.Id;
        var result = await _workspaceAppService.RegisterAsync(input);

        using (_currentWorkspace.Change(result.WorkspaceId))
        {
            var subscription = await WithUnitOfWorkAsync(() => _subscriptionRepository.FirstAsync(s => s.WorkspaceId == result.WorkspaceId));
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
        (await FindWorkspaceAsync("ghostplan")).ShouldBeNull();
    }

    [Fact]
    public async Task Register_Should_Refuse_A_Paid_Plan_Without_A_Trial()
    {
        await WithUnitOfWorkAsync(() => _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Enterprise", 25m, 500, 0)));

        var exception = await Should.ThrowAsync<AbpValidationException>(
            () => _workspaceAppService.RegisterAsync(SignUp("bigco", planName: "Enterprise")));

        exception.ValidationErrors.ShouldContain(e => e.MemberNames.Contains(nameof(RegisterWorkspaceDto.PlanName)));
        (await FindWorkspaceAsync("bigco")).ShouldBeNull();
    }

    [Theory]
    [InlineData("a")]
    [InlineData("12345")]
    public async Task Register_Should_Enforce_The_Password_Policy(string password)
    {
        var exception = await Should.ThrowAsync<AbpValidationException>(
            () => _workspaceAppService.RegisterAsync(SignUp("weakco", password: password)));

        exception.ValidationErrors.ShouldNotBeEmpty();
        exception.ValidationErrors.ShouldAllBe(e => e.MemberNames.Contains(nameof(RegisterWorkspaceDto.Password)));
        (await FindWorkspaceAsync("weakco")).ShouldBeNull();
        (await FindUserByEmailAsync("owner@weakco.test")).ShouldBeNull();
    }

    [Fact]
    public async Task Register_Should_Keep_The_Password_Out_Of_The_Audit_Log()
    {
        const string password = "Sup3r-Secret-Owner!";
        await _workspaceAppService.RegisterAsync(SignUp("audited", password: password));

        var logs = await WithUnitOfWorkAsync(() => GetRequiredService<IAuditLogRepository>().GetListAsync(includeDetails: true));
        var action = logs.SelectMany(l => l.Actions)
            .Where(a => a.MethodName == nameof(IWorkspaceAppService.RegisterAsync))
            .ShouldHaveSingleItem();

        action.Parameters.ShouldContain("audited");
        action.Parameters.ShouldContain("owner@audited.test");
        action.Parameters.ShouldNotContain(password);
    }

    [Fact]
    public async Task Register_Should_Reject_A_Workspace_Name_In_Use_Ignoring_Case()
    {
        await _workspaceAppService.RegisterAsync(SignUp("twice"));

        var input = SignUp("TWICE");
        input.Email = "another@twice.test";
        var exception = await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.RegisterAsync(input));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.DuplicateWorkspaceName);
        (await FindUserByEmailAsync("another@twice.test")).ShouldBeNull(); // nothing was created
    }

    [Fact]
    public async Task Register_Should_Trim_The_Workspace_Name()
    {
        var result = await _workspaceAppService.RegisterAsync(SignUp("  spaced  "));

        result.WorkspaceName.ShouldBe("spaced");
        (await FindWorkspaceAsync("spaced")).ShouldNotBeNull();
    }

    // ---- My workspaces ----

    [Fact]
    public async Task Mine_Should_List_The_Active_Memberships_By_Name_With_Role_And_Plan()
    {
        var userId = await CreateUserAsync("multi@ws.test", WorkspaceRoles.Member);
        var zeta = await _workspaceAppService.RegisterAsync(SignUp("zeta", planName: "Starter"));
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Owner, zeta.WorkspaceId);
        var gone = await ServiceProvider.CreateWorkspaceAsync("alpha-gone");
        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Member, gone);
        await WithUnitOfWorkAsync(async () =>
        {
            var manager = GetRequiredService<WorkspaceMembershipManager>();
            await manager.RemoveAsync((await manager.FindAsync(gone, userId))!);
        });

        using (ActAs(userId))
        using (_currentWorkspace.Change(null))
        {
            var mine = await _workspaceAppService.GetMineAsync();

            mine.Select(w => w.Name).ShouldBe(new[] { TrackerTestConsts.WorkspaceName, "zeta" });
            mine[0].Role.ShouldBe(WorkspaceRoles.Member);
            mine[0].IsOwner.ShouldBeFalse();
            mine[0].PlanName.ShouldBeNull();
            mine[1].Role.ShouldBe(WorkspaceRoles.Owner);
            mine[1].IsOwner.ShouldBeTrue();
            mine[1].PlanName.ShouldBe("Starter");
        }
    }

    [Fact]
    public async Task Create_Should_Make_The_Caller_Owner_Of_A_New_Workspace()
    {
        var userId = await CreateUserAsync("creator@ws.test");

        using (ActAs(userId))
        using (_currentWorkspace.Change(null))
        {
            var created = await _workspaceAppService.CreateAsync(new CreateWorkspaceDto { Name = " Second Co " });

            created.Name.ShouldBe("Second Co");
            (await ServiceProvider.FindWorkspaceMemberAsync(userId, created.Id))!.Role.ShouldBe(WorkspaceRoles.Owner);
            (await _workspaceAppService.GetMineAsync()).ShouldHaveSingleItem().Id.ShouldBe(created.Id);

            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.CreateAsync(new CreateWorkspaceDto { Name = "second co" })))
                .Code.ShouldBe(TrackerDomainErrorCodes.DuplicateWorkspaceName);
        }
    }

    [Fact]
    public async Task Leaving_Should_End_The_Membership_And_Its_Project_Memberships_But_Not_For_The_Last_Owner()
    {
        var ownerId = await CreateUserAsync("owner@leave.test", WorkspaceRoles.Owner);
        var memberId = await CreateUserAsync("member@leave.test", WorkspaceRoles.Member);
        var projectId = await WithUnitOfWorkAsync(async () =>
        {
            var project = await GetRequiredService<IRepository<Project, Guid>>()
                .InsertAsync(new Project(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, "Leave"));
            await GetRequiredService<IRepository<ProjectMember, Guid>>()
                .InsertAsync(new ProjectMember(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, project.Id, memberId));
            return project.Id;
        });

        using (ActAs(memberId))
        using (_currentWorkspace.Change(null))
        {
            await _workspaceAppService.LeaveAsync(TrackerTestConsts.WorkspaceId);
            (await _workspaceAppService.GetMineAsync()).ShouldBeEmpty();
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.LeaveAsync(TrackerTestConsts.WorkspaceId)))
                .Code.ShouldBe(TrackerDomainErrorCodes.NotWorkspaceMember);
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(memberId))!.Status.ShouldBe(WorkspaceMemberStatus.Removed);
        (await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<ProjectMember, Guid>>()
            .GetListAsync(m => m.ProjectId == projectId))).ShouldBeEmpty();
        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().GetByIdAsync(memberId))).IsActive.ShouldBeTrue();

        using (ActAs(ownerId))
        using (_currentWorkspace.Change(null))
        {
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.LeaveAsync(TrackerTestConsts.WorkspaceId)))
                .Code.ShouldBe(TrackerDomainErrorCodes.LastOwnerCannotLeave);
        }

        var secondOwner = await CreateUserAsync("owner2@leave.test", WorkspaceRoles.Owner);
        using (ActAs(ownerId))
        using (_currentWorkspace.Change(null))
        {
            await _workspaceAppService.LeaveAsync(TrackerTestConsts.WorkspaceId);
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(secondOwner))!.IsOwner.ShouldBeTrue();
    }

    // ---- Invitations ----

    private async Task<string> InviteAsync(string email, Guid? projectId = null, string role = ProjectMemberRoles.Worker)
    {
        // The (fake) signed-in caller invites as an owner.
        await ServiceProvider.AddWorkspaceMemberAsync(Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d"), WorkspaceRoles.Owner);
        var result = await GetRequiredService<ITeamAppService>().InviteMemberAsync(new InviteMemberDto
        {
            Email = email,
            ProjectId = projectId,
            Role = role
        });
        result.Added.ShouldBeFalse();
        result.InvitePath.ShouldNotBeNull().ShouldStartWith("/invite/");
        return result.InvitePath!["/invite/".Length..];
    }

    [Fact]
    public async Task An_Invitation_Should_Be_Readable_Anonymously_And_Accepted_By_The_Invited_Email_Only()
    {
        var token = await InviteAsync("newcomer@invite.test");

        using (_currentWorkspace.Change(null))
        {
            var info = await _workspaceAppService.GetInvitationAsync(token);
            info.WorkspaceName.ShouldBe(TrackerTestConsts.WorkspaceName);
            info.Email.ShouldBe("newcomer@invite.test");
            info.Role.ShouldBe(WorkspaceRoles.Member);
            info.Expired.ShouldBeFalse();

            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.GetInvitationAsync("not-a-token")))
                .Code.ShouldBe(TrackerDomainErrorCodes.InvitationNotFound);
        }

        var stranger = await CreateUserAsync("stranger@invite.test");
        using (ActAs(stranger))
        using (_currentWorkspace.Change(null))
        {
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.AcceptInvitationAsync(token)))
                .Code.ShouldBe(TrackerDomainErrorCodes.InvitationEmailMismatch);
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(stranger)).ShouldBeNull();

        var invitee = await CreateUserAsync("NEWCOMER@invite.test");
        using (ActAs(invitee))
        using (_currentWorkspace.Change(null))
        {
            var accepted = await _workspaceAppService.AcceptInvitationAsync(token);
            accepted.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
            accepted.Role.ShouldBe(WorkspaceRoles.Member);

            (await _workspaceAppService.GetInvitationAsync(token)).Expired.ShouldBeTrue();
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.AcceptInvitationAsync(token)))
                .Code.ShouldBe(TrackerDomainErrorCodes.InvitationExpired);
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(invitee))!.IsActive.ShouldBeTrue();
    }

    [Fact]
    public async Task Signing_Up_With_An_Invitation_Should_Join_The_Inviting_Workspace_And_Its_Project()
    {
        var projectId = await WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<Project, Guid>>()
            .InsertAsync(new Project(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, "Invited"))).Id);
        var token = await InviteAsync("joiner@invite.test", projectId, ProjectMemberRoles.Manager);

        var mismatch = new RegisterWorkspaceDto { Email = "other@invite.test", Password = "1q2w3E*join", InviteToken = token };
        (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.RegisterAsync(mismatch)))
            .Code.ShouldBe(TrackerDomainErrorCodes.InvitationEmailMismatch);
        (await FindUserByEmailAsync("other@invite.test")).ShouldBeNull();

        var result = await _workspaceAppService.RegisterAsync(new RegisterWorkspaceDto
        {
            Email = "joiner@invite.test",
            Password = "1q2w3E*join",
            InviteToken = token
        });

        result.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
        result.WorkspaceName.ShouldBe(TrackerTestConsts.WorkspaceName);
        var user = (await FindUserByEmailAsync("joiner@invite.test")).ShouldNotBeNull();
        (await ServiceProvider.FindWorkspaceMemberAsync(user.Id))!.Role.ShouldBe(WorkspaceRoles.Manager); // the Manager project role
        (await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<ProjectMember, Guid>>()
                .GetListAsync(m => m.UserId == user.Id)))
            .ShouldHaveSingleItem().Role.ShouldBe(ProjectMemberRoles.Manager);
        (await WithUnitOfWorkAsync(() => _workspaceRepository.CountAsync())).ShouldBe(1); // no new workspace
    }

    // ---- Workspace settings ----

    [Fact]
    public async Task Settings_Should_Start_From_The_Platform_Defaults_And_Be_Readable_By_Any_Member()
    {
        using (_currentWorkspace.Change(Guid.NewGuid()))
        using (ActAs(Guid.NewGuid()))
        {
            var settings = await _workspaceAppService.GetSettingsAsync();

            settings.DefaultTimeZone.ShouldBeNull();
            settings.NewProjectDefaults.ShouldBeEquivalentTo(new NewProjectDefaultsDto());
        }
    }

    [Fact]
    public async Task The_Owner_Should_Change_The_Settings_Of_Their_Workspace_Only()
    {
        var owner = await CreateUserAsync("settings-owner@ws.test", WorkspaceRoles.Owner);
        using (ActAs(owner))
        {
            var updated = await _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka"));

            updated.ShouldBeEquivalentTo(Settings("Asia/Dhaka"));
            (await _workspaceAppService.GetSettingsAsync()).ShouldBeEquivalentTo(Settings("Asia/Dhaka"));
        }

        using (_currentWorkspace.Change(Guid.NewGuid()))
        {
            (await _workspaceAppService.GetSettingsAsync()).DefaultTimeZone.ShouldBeNull();
        }
    }

    [Fact]
    public async Task A_Windows_Time_Zone_Should_Be_Stored_As_Its_Iana_Id_And_A_Blank_One_Should_Unset_It()
    {
        var owner = await CreateUserAsync("tz-owner@ws.test", WorkspaceRoles.Owner);
        using (ActAs(owner))
        {
            (await _workspaceAppService.UpdateSettingsAsync(Settings("Bangladesh Standard Time"))).DefaultTimeZone.ShouldBe("Asia/Dhaka");
            (await _workspaceAppService.UpdateSettingsAsync(Settings("  "))).DefaultTimeZone.ShouldBeNull();
        }
    }

    [Fact]
    public async Task Only_The_Workspace_Owner_Should_Change_The_Settings()
    {
        var manager = await CreateUserAsync("settings-manager@ws.test", WorkspaceRoles.Manager);
        using (ActAs(manager))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka")));
            (await _workspaceAppService.GetSettingsAsync()).DefaultTimeZone.ShouldBeNull();
        }

        // An owner of another workspace is no owner here.
        var otherOwner = await CreateUserAsync("other-owner@ws.test", WorkspaceRoles.Owner, Guid.NewGuid());
        using (ActAs(otherOwner))
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka")));
        }
    }

    [Fact]
    public async Task Settings_Without_A_Workspace_Should_Require_One()
    {
        using (_currentWorkspace.Change(null))
        {
            (await Should.ThrowAsync<BusinessException>(() => _workspaceAppService.UpdateSettingsAsync(Settings("Asia/Dhaka"))))
                .Code.ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);
        }
    }

    [Fact]
    public async Task An_Unknown_Time_Zone_Should_Be_A_Validation_Error()
    {
        var owner = await CreateUserAsync("bad-tz@ws.test", WorkspaceRoles.Owner);
        using (ActAs(owner))
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
        var owner = await CreateUserAsync($"range-{intervalMinutes}-{idleThresholdMinutes}@ws.test", WorkspaceRoles.Owner);
        using (ActAs(owner))
        {
            await Should.ThrowAsync<AbpValidationException>(() => _workspaceAppService.UpdateSettingsAsync(
                Settings("Asia/Dhaka", intervalMinutes, idleThresholdMinutes)));
        }
    }

    // ---- My preferences ----

    [Fact]
    public async Task My_Time_Zone_Should_Be_Mine_Alone_And_Can_Be_Cleared()
    {
        var alice = Guid.NewGuid();
        using (_currentWorkspace.Change(null))
        {
            using (ActAs(alice))
            {
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
                (await _workspaceAppService.UpdateMyPreferencesAsync(new MyPreferencesDto { TimeZone = "Europe/London" }))
                    .TimeZone.ShouldBe("Europe/London");
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBe("Europe/London");
            }

            using (ActAs(Guid.NewGuid()))
            {
                (await _workspaceAppService.GetMyPreferencesAsync()).TimeZone.ShouldBeNull();
            }

            using (ActAs(alice))
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
