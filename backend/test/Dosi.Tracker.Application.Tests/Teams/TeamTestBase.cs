using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// Shared set-up for the team tests. The test principal (FakeCurrentPrincipalAccessor) is given an account and a
/// membership of the test workspace in <see cref="CallerRole"/> before every test: an owner by default, since the
/// owner-only rules are enforced in the service and not by the (always-allow) test authorization.
/// </summary>
public abstract class TeamTestBase<TStartupModule> : TrackerApplicationTestBase<TStartupModule>, IAsyncLifetime
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    protected static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    protected ITeamAppService TeamAppService { get; }
    protected IProjectAppService ProjectAppService { get; }
    protected IGuidGenerator GuidGenerator { get; }

    protected TeamTestBase()
    {
        TeamAppService = GetRequiredService<ITeamAppService>();
        ProjectAppService = GetRequiredService<IProjectAppService>();
        GuidGenerator = GetRequiredService<IGuidGenerator>();
    }

    /// <summary>The caller's workspace role (<see cref="WorkspaceRoles"/>), or null for a caller without an account.</summary>
    protected virtual string? CallerRole => WorkspaceRoles.Owner;

    public virtual async Task InitializeAsync()
    {
        if (CallerRole != null)
        {
            await CreateUserAsync("caller", CallerRole, CurrentUserId);
        }
    }

    public virtual Task DisposeAsync()
    {
        return Task.CompletedTask;
    }

    /// <summary>An account that is an active member of the test workspace with <paramref name="role"/> (a plain member
    /// by default), unless <paramref name="member"/> is false.</summary>
    protected async Task<IdentityUser> CreateUserAsync(
        string userName,
        string? role = null,
        Guid? id = null,
        bool emailConfirmed = false,
        string? email = null,
        bool member = true)
    {
        var user = await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var created = new IdentityUser(id ?? GuidGenerator.Create(), userName, email ?? $"{userName}@acme.test")
            {
                Name = userName.ToUpperInvariant()
            };
            created.SetEmailConfirmed(emailConfirmed);
            (await userManager.CreateAsync(created)).Succeeded.ShouldBeTrue();
            return created;
        });

        if (member)
        {
            await ServiceProvider.AddWorkspaceMemberAsync(user.Id, role ?? WorkspaceRoles.Member);
        }

        return user;
    }

    protected Task<IdentityUser?> FindUserAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<IIdentityUserRepository>().FindAsync(userId));
    }

    /// <summary>Whether the user is an active member of the test workspace with this role.</summary>
    protected async Task<bool> IsInRoleAsync(Guid userId, string role)
    {
        var member = await ServiceProvider.FindWorkspaceMemberAsync(userId);
        return member is { IsActive: true } && member.Role == role;
    }

    protected async Task<bool> IsActiveMemberAsync(Guid userId)
    {
        return (await ServiceProvider.FindWorkspaceMemberAsync(userId))?.IsActive == true;
    }

    protected async Task<Guid> CreateProjectAsync(string title = "Team Test Project")
    {
        var project = await ProjectAppService.CreateAsync(new CreateUpdateProjectDto
        {
            Title = title,
            Color = "#006bff",
            IntervalMinutes = 10
        });
        return project.Id;
    }

    /// <summary>The member's rate history on the project, oldest first.</summary>
    protected Task<List<ProjectMemberRate>> GetRatesAsync(Guid projectId, Guid userId)
    {
        return WithUnitOfWorkAsync(async () =>
            (await GetRequiredService<IRepository<ProjectMemberRate, Guid>>()
                .GetListAsync(r => r.ProjectId == projectId && r.UserId == userId))
            .OrderBy(r => r.EffectiveFrom)
            .ThenBy(r => r.CreationTime)
            .ToList());
    }

    protected Task<List<ProjectMember>> GetMembershipsOfUserAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(() =>
            GetRequiredService<IRepository<ProjectMember, Guid>>().GetListAsync(m => m.UserId == userId));
    }

    protected Task<ProjectMemberDto> AddAsync(Guid projectId, Guid userId, string role = ProjectMemberRoles.Worker, decimal hourlyRate = 0m)
    {
        return TeamAppService.AddMemberAsync(new CreateProjectMemberDto
        {
            ProjectId = projectId,
            UserId = userId,
            Role = role,
            HourlyRate = hourlyRate
        });
    }
}
