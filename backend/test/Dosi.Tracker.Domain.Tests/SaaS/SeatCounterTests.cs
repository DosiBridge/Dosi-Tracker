using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class SeatCounterTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly SeatCounter _seatCounter;
    private readonly IRepository<Project, Guid> _projects;
    private readonly IRepository<ProjectMember, Guid> _members;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly IdentityUserManager _userManager;
    private readonly ICurrentTenant _currentTenant;
    private readonly IGuidGenerator _guidGenerator;

    protected SeatCounterTests()
    {
        _seatCounter = GetRequiredService<SeatCounter>();
        _projects = GetRequiredService<IRepository<Project, Guid>>();
        _members = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private async Task<T> InWorkspaceAsync<T>(Guid? tenantId, Func<Task<T>> action)
    {
        using (_currentTenant.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(action);
        }
    }

    private async Task InWorkspaceAsync(Guid? tenantId, Func<Task> action)
    {
        using (_currentTenant.Change(tenantId))
        {
            await WithUnitOfWorkAsync(action);
        }
    }

    private Task<Guid> CreateProjectAsync(Guid? tenantId)
    {
        return InWorkspaceAsync(tenantId, async () =>
            (await _projects.InsertAsync(new Project(_guidGenerator.Create(), tenantId, "Seats"))).Id);
    }

    private Task AddMemberAsync(Guid? tenantId, Guid projectId, Guid userId, string role = "Worker")
    {
        return InWorkspaceAsync(tenantId, () =>
            _members.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, projectId, userId, role)));
    }

    private Task<Guid> CreateUserAsync(Guid? tenantId, string userName)
    {
        return InWorkspaceAsync(tenantId, async () =>
        {
            var user = new IdentityUser(_guidGenerator.Create(), userName, $"{userName}@seats.test", tenantId);
            (await _userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
            return user.Id;
        });
    }

    /// <summary>A new person of the workspace (an active account) as a worker on the project; returns their id.</summary>
    private async Task<Guid> AddSeatHolderAsync(Guid? tenantId, Guid projectId)
    {
        var userId = await CreateUserAsync(tenantId, $"member-{Guid.NewGuid():N}");
        await AddMemberAsync(tenantId, projectId, userId);
        return userId;
    }

    private Task<int> CountAsync(Guid? tenantId)
    {
        return InWorkspaceAsync(tenantId, () => _seatCounter.CountAsync());
    }

    private async Task SubscribeAsync(Guid tenantId, int maxSeats)
    {
        var plan = await InWorkspaceAsync(null, () =>
            _plans.InsertAsync(new Plan(_guidGenerator.Create(), $"Seats {Guid.NewGuid():N}", 5m, maxSeats, 0)));
        await InWorkspaceAsync(tenantId, () => _subscriptions.InsertAsync(
            new Subscription(_guidGenerator.Create(), tenantId, plan.Id) { Status = Subscription.ActiveStatus }));
    }

    [Fact]
    public async Task Each_Distinct_Member_Of_An_Existing_Project_Should_Take_One_Seat()
    {
        var tenantId = Guid.NewGuid();
        var website = await CreateProjectAsync(tenantId);
        var app = await CreateProjectAsync(tenantId);
        var alice = await CreateUserAsync(tenantId, "alice");
        var bob = await CreateUserAsync(tenantId, "bob");
        await AddMemberAsync(tenantId, website, alice);
        await AddMemberAsync(tenantId, app, alice); // a second project: still one seat
        await AddMemberAsync(tenantId, website, bob);

        (await CountAsync(tenantId)).ShouldBe(2);
    }

    [Fact]
    public async Task Members_Of_A_Deleted_Project_Should_Not_Take_A_Seat()
    {
        var tenantId = Guid.NewGuid();
        var kept = await CreateProjectAsync(tenantId);
        var deleted = await CreateProjectAsync(tenantId);
        var alice = await CreateUserAsync(tenantId, "alice");
        var bob = await CreateUserAsync(tenantId, "bob");
        await AddMemberAsync(tenantId, kept, alice);
        await AddMemberAsync(tenantId, deleted, alice);
        await AddMemberAsync(tenantId, deleted, bob);

        await InWorkspaceAsync(tenantId, () => _projects.DeleteAsync(deleted));

        (await CountAsync(tenantId)).ShouldBe(1); // alice still holds a seat through the kept project
    }

    [Fact]
    public async Task A_Deleted_Or_Deactivated_Account_Should_Free_Its_Seat()
    {
        var tenantId = Guid.NewGuid();
        var project = await CreateProjectAsync(tenantId);
        var kept = await CreateUserAsync(tenantId, "kept");
        var deleted = await CreateUserAsync(tenantId, "deleted");
        var deactivated = await CreateUserAsync(tenantId, "deactivated");
        foreach (var userId in new[] { kept, deleted, deactivated })
        {
            await AddMemberAsync(tenantId, project, userId);
        }

        await InWorkspaceAsync(tenantId, async () =>
        {
            (await _userManager.DeleteAsync(await _userManager.GetByIdAsync(deleted))).Succeeded.ShouldBeTrue();
            var user = await _userManager.GetByIdAsync(deactivated);
            user.SetIsActive(false);
            (await _userManager.UpdateAsync(user)).Succeeded.ShouldBeTrue();
        });

        (await CountAsync(tenantId)).ShouldBe(1);
        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(deactivated))).ShouldBeFalse();
    }

    [Fact]
    public async Task A_Membership_Without_An_Account_In_The_Workspace_Should_Not_Take_A_Seat()
    {
        // A stray user id (e.g. added before members were validated), or the account of another workspace, bills nothing.
        var tenantId = Guid.NewGuid();
        var otherTenantId = Guid.NewGuid();
        var project = await CreateProjectAsync(tenantId);
        var outsider = await CreateUserAsync(otherTenantId, "outsider");
        await AddMemberAsync(tenantId, project, Guid.NewGuid());
        await AddMemberAsync(tenantId, project, outsider);
        await AddSeatHolderAsync(tenantId, project);

        (await CountAsync(tenantId)).ShouldBe(1);
        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(outsider))).ShouldBeFalse();
    }

    [Fact]
    public async Task Clients_Should_Not_Take_A_Seat()
    {
        var tenantId = Guid.NewGuid();
        var website = await CreateProjectAsync(tenantId);
        var app = await CreateProjectAsync(tenantId);
        var client = await CreateUserAsync(tenantId, "client");
        var spaced = await CreateUserAsync(tenantId, "spaced");
        var twoHats = await CreateUserAsync(tenantId, "twohats");
        await AddMemberAsync(tenantId, website, client, "Client");
        await AddMemberAsync(tenantId, app, client, "Client");
        await AddMemberAsync(tenantId, website, spaced, " client "); // any case, surrounding spaces ignored
        await AddMemberAsync(tenantId, website, twoHats, "Client");
        await AddMemberAsync(tenantId, app, twoHats); // tracks time on another project: one seat

        (await CountAsync(tenantId)).ShouldBe(1);
        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(client))).ShouldBeFalse();
        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(twoHats))).ShouldBeTrue();
    }

    [Fact]
    public async Task Only_The_Current_Workspaces_Members_Should_Count()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();
        var projectA = await CreateProjectAsync(tenantA);
        var projectB = await CreateProjectAsync(tenantB);
        await AddSeatHolderAsync(tenantA, projectA);
        await AddSeatHolderAsync(tenantB, projectB);
        await AddSeatHolderAsync(tenantB, projectB);

        (await CountAsync(tenantA)).ShouldBe(1);
        (await CountAsync(tenantB)).ShouldBe(2);
    }

    [Fact]
    public async Task Count_By_Tenant_Should_Group_Workspaces_And_Ignore_Host_Memberships()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();
        var projectA = await CreateProjectAsync(tenantA);
        var secondProjectA = await CreateProjectAsync(tenantA);
        var projectB = await CreateProjectAsync(tenantB);
        var hostProject = await CreateProjectAsync(null);
        var onTwoProjects = await AddSeatHolderAsync(tenantA, projectA);
        await AddMemberAsync(tenantA, secondProjectA, onTwoProjects);
        await AddSeatHolderAsync(tenantA, projectA);
        await AddSeatHolderAsync(tenantB, projectB);
        await AddSeatHolderAsync(null, hostProject);

        var seats = await InWorkspaceAsync(null, () => _seatCounter.CountByTenantAsync());

        seats.Count.ShouldBe(2);
        seats[tenantA].ShouldBe(2);
        seats[tenantB].ShouldBe(1);
    }

    [Fact]
    public async Task HoldsSeat_Should_Tell_Seat_Holders_Apart()
    {
        var tenantId = Guid.NewGuid();
        var project = await CreateProjectAsync(tenantId);
        var member = await AddSeatHolderAsync(tenantId, project);

        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(member))).ShouldBeTrue();
        (await InWorkspaceAsync(tenantId, () => _seatCounter.HoldsSeatAsync(Guid.NewGuid()))).ShouldBeFalse();
    }

    [Fact]
    public async Task A_New_Seat_Should_Be_Refused_At_The_Plan_Limit_But_Not_For_A_Seat_Holder()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, maxSeats: 2);
        var project = await CreateProjectAsync(tenantId);
        var first = await AddSeatHolderAsync(tenantId, project);

        // One seat left: a newcomer may join, with or without an account yet.
        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(Guid.NewGuid()));
        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(null));
        await AddSeatHolderAsync(tenantId, project);

        var exception = await Should.ThrowAsync<BusinessException>(
            () => InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(Guid.NewGuid())));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        exception.Data["maxSeats"].ShouldBe(2);
        (await Should.ThrowAsync<BusinessException>(
                () => InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(null))))
            .Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);

        // A seat holder joining another project takes no new seat.
        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(first));
    }

    [Fact]
    public async Task A_Seat_Freed_By_Deleting_A_Project_Should_Be_Available_Again()
    {
        var tenantId = Guid.NewGuid();
        await SubscribeAsync(tenantId, maxSeats: 1);
        var project = await CreateProjectAsync(tenantId);
        await AddSeatHolderAsync(tenantId, project);
        await Should.ThrowAsync<BusinessException>(
            () => InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(Guid.NewGuid())));

        await InWorkspaceAsync(tenantId, () => _projects.DeleteAsync(project));

        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(Guid.NewGuid()));
    }

    [Fact]
    public async Task A_Workspace_Without_A_Subscription_Should_Have_No_Seat_Limit()
    {
        var tenantId = Guid.NewGuid();
        var project = await CreateProjectAsync(tenantId);
        for (var i = 0; i < 5; i++)
        {
            await AddSeatHolderAsync(tenantId, project);
        }

        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckSeatAvailableAsync(Guid.NewGuid()));
    }

    [Fact]
    public async Task A_Plan_Smaller_Than_The_Occupied_Seats_Should_Not_Fit()
    {
        var tenantId = Guid.NewGuid();
        var project = await CreateProjectAsync(tenantId);
        await AddSeatHolderAsync(tenantId, project);
        await AddSeatHolderAsync(tenantId, project);

        var exactFit = new Plan(Guid.NewGuid(), "Two", 5m, 2, 0);
        var tooSmall = new Plan(Guid.NewGuid(), "One", 5m, 1, 0);
        var unlimited = new Plan(Guid.NewGuid(), "Unlimited", 5m, 0, 0);

        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckFitsPlanAsync(exactFit));
        await InWorkspaceAsync(tenantId, () => _seatCounter.CheckFitsPlanAsync(unlimited));
        var exception = await Should.ThrowAsync<BusinessException>(
            () => InWorkspaceAsync(tenantId, () => _seatCounter.CheckFitsPlanAsync(tooSmall)));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.SeatLimitReached);
        exception.Data["seatsUsed"].ShouldBe(2);
    }
}
