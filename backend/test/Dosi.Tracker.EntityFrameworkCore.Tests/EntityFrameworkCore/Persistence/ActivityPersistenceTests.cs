using System;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

/* Schema-level guarantees of AppActivities (the indexes and columns of the AgentDataV2 migration),
 * checked against the real database rather than through the application service. */
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class ActivityPersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly DateTime Start = new(2026, 7, 20, 9, 0, 0, DateTimeKind.Utc);

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly ICurrentWorkspace _currentWorkspace;

    public ActivityPersistenceTests()
    {
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    private Task InsertAsync(Guid tenantId, Guid userId, Guid clientActivityId, DateTime start)
    {
        return WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(
            new Activity(Guid.NewGuid(), tenantId, userId, Guid.NewGuid(), clientActivityId, start, start.AddMinutes(10)),
            autoSave: true));
    }

    [Fact]
    public async Task A_Member_Cannot_Store_Two_Blocks_With_The_Same_ClientActivityId()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var clientId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await InsertAsync(tenantId, userId, clientId, Start);

            // The unique (TenantId, UserId, ClientActivityId) index is the backstop for concurrent retries.
            await Should.ThrowAsync<DbUpdateException>(() => InsertAsync(tenantId, userId, clientId, Start.AddHours(1)));

            (await WithUnitOfWorkAsync(() => _activityRepository.CountAsync(a => a.ClientActivityId == clientId)))
                .ShouldBe(1);
        }
    }

    [Fact]
    public async Task Different_Members_Of_A_Workspace_Can_Store_The_Same_ClientActivityId()
    {
        // Uniqueness is per member; refusing an id that another member already used is the ingest service's job
        // (Tracker:ActivityClientIdConflict), not the index's.
        var tenantId = Guid.NewGuid();
        var clientId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await InsertAsync(tenantId, Guid.NewGuid(), clientId, Start);
            await InsertAsync(tenantId, Guid.NewGuid(), clientId, Start);

            (await WithUnitOfWorkAsync(() => _activityRepository.CountAsync(a => a.ClientActivityId == clientId)))
                .ShouldBe(2);
        }
    }

    [Fact]
    public async Task Should_Persist_The_Block_Context_And_An_Unmeasured_Productivity()
    {
        var tenantId = Guid.NewGuid();
        var deviceId = Guid.NewGuid();
        var id = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(
                new Activity(id, tenantId, Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), Start, Start.AddMinutes(10))
                {
                    Productivity = null,
                    Platform = "windows",
                    OsVersion = "Windows 11 Pro 23H2 (10.0.22631)",
                    AgentVersion = "0.2.0",
                    DeviceId = deviceId,
                    DeviceName = "DESKTOP-42",
                    TimeZone = "Asia/Dhaka",
                    UtcOffsetMinutes = 360,
                    IdleSeconds = 90,
                    EndReason = "lock",
                    ClockSkewSeconds = 12,
                    InjectedInputEvents = 3
                }));

            var stored = await WithUnitOfWorkAsync(() => _activityRepository.GetAsync(id));

            stored.Productivity.ShouldBeNull();
            stored.Platform.ShouldBe("windows");
            stored.OsVersion.ShouldBe("Windows 11 Pro 23H2 (10.0.22631)");
            stored.AgentVersion.ShouldBe("0.2.0");
            stored.DeviceId.ShouldBe(deviceId);
            stored.DeviceName.ShouldBe("DESKTOP-42");
            stored.TimeZone.ShouldBe("Asia/Dhaka");
            stored.UtcOffsetMinutes.ShouldBe(360);
            stored.IdleSeconds.ShouldBe(90);
            stored.EndReason.ShouldBe("lock");
            stored.ClockSkewSeconds.ShouldBe(12);
            stored.InjectedInputEvents.ShouldBe(3);
        }
    }
}
