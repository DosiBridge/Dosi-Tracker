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
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// Shared set-up for the team tests. The test principal (FakeCurrentPrincipalAccessor) is given an Identity account in
/// <see cref="CallerRole"/> before every test: a workspace owner by default, since the owner-only rules are enforced in
/// the service and not by the (always-allow) test authorization.
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

    /// <summary>The caller's Identity role: <see cref="TrackerRoles.Admin"/> (workspace owner),
    /// <see cref="TrackerRoles.Manager"/> (team lead), or null for a caller without an Identity account.</summary>
    protected virtual string? CallerRole => TrackerRoles.Admin;

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

    /// <summary>An active user of the (host) workspace, optionally in an Identity role.</summary>
    protected Task<IdentityUser> CreateUserAsync(
        string userName,
        string? role = null,
        Guid? id = null,
        bool emailConfirmed = false,
        string? email = null)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var user = new IdentityUser(id ?? GuidGenerator.Create(), userName, email ?? $"{userName}@acme.test")
            {
                Name = userName.ToUpperInvariant()
            };
            user.SetEmailConfirmed(emailConfirmed);
            (await userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();

            if (role != null)
            {
                (await userManager.AddToRoleAsync(user, role)).Succeeded.ShouldBeTrue();
            }

            return user;
        });
    }

    protected Task<IdentityUser?> FindUserAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<IIdentityUserRepository>().FindAsync(userId));
    }

    protected Task<bool> IsInRoleAsync(Guid userId, string role)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var roles = await GetRequiredService<IIdentityUserRepository>().GetRoleNamesAsync(userId);
            return roles.Contains(role);
        });
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
