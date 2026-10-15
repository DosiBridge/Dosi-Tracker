using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// Changes that can take a seat are serialized per workspace: adding, inviting and re-roling members hold the
/// workspace's seat lock while they check the plan's limit and save, so two concurrent adds cannot both take the last
/// seat. (The in-memory test database cannot run two requests at once, so the lock itself is observed here.)
/// </summary>
public abstract class TeamSeatLockTests<TStartupModule> : TeamTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly RecordingLock _locks = new();

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.Replace(ServiceDescriptor.Singleton<IAbpDistributedLock>(_locks));
    }

    [Fact]
    public async Task Adding_Inviting_And_Changing_Members_Should_Hold_The_Workspace_Seat_Lock()
    {
        var projectId = await CreateProjectAsync();
        var worker = await CreateUserAsync("worker");
        _locks.Acquired.Clear();

        await AddAsync(projectId, worker.Id);
        await TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "new@acme.test" });
        await TeamAppService.UpdateMemberAsync(projectId, worker.Id, new UpdateProjectMemberDto { Role = ProjectMemberRoles.Client });

        _locks.Acquired.ShouldBe(Enumerable.Repeat(SeatCounter.SeatLockName(TrackerTestConsts.WorkspaceId), 3));
        _locks.Held.ShouldBe(0); // released again
    }

    [Fact]
    public async Task A_Change_That_Cannot_Get_The_Seat_Lock_Should_Change_Nothing()
    {
        var projectId = await CreateProjectAsync();
        var worker = await CreateUserAsync("worker");
        _locks.Refuse = true;

        await Should.ThrowAsync<AbpException>(() => AddAsync(projectId, worker.Id));
        await Should.ThrowAsync<AbpException>(() =>
            TeamAppService.InviteMemberAsync(new InviteMemberDto { ProjectId = projectId, Email = "new@acme.test" }));

        (await GetMembershipsOfUserAsync(worker.Id)).ShouldBeEmpty();
        (await WithUnitOfWorkAsync(() => GetRequiredService<IdentityUserManager>().FindByEmailAsync("new@acme.test")))
            .ShouldBeNull();
    }

    [Fact]
    public void Every_Workspace_Should_Have_Its_Own_Seat_Lock()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();

        SeatCounter.SeatLockName(tenantA).ShouldNotBe(SeatCounter.SeatLockName(tenantB));
        SeatCounter.SeatLockName(tenantA).ShouldNotBe(SeatCounter.SeatLockName(TrackerTestConsts.WorkspaceId));
    }

    /// <summary>A lock that grants every request (unless told to refuse) and records what was taken.</summary>
    private sealed class RecordingLock : IAbpDistributedLock
    {
        public List<string> Acquired { get; } = new();

        public int Held { get; private set; }

        public bool Refuse { get; set; }

        public Task<IAbpDistributedLockHandle?> TryAcquireAsync(
            string name, TimeSpan timeout = default, CancellationToken cancellationToken = default)
        {
            if (Refuse)
            {
                return Task.FromResult<IAbpDistributedLockHandle?>(null);
            }

            Acquired.Add(name);
            Held++;
            return Task.FromResult<IAbpDistributedLockHandle?>(new Handle(this));
        }

        private sealed class Handle : IAbpDistributedLockHandle
        {
            private readonly RecordingLock _owner;

            public Handle(RecordingLock owner)
            {
                _owner = owner;
            }

            public ValueTask DisposeAsync()
            {
                _owner.Held--;
                return ValueTask.CompletedTask;
            }
        }
    }
}
