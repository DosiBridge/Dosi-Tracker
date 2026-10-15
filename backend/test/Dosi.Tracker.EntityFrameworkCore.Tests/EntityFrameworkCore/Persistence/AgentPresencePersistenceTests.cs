using System;
using System.Threading.Tasks;
using Dosi.Tracker.Presence;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class AgentPresencePersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly DateTime Now = new(2026, 10, 5, 9, 30, 0, DateTimeKind.Utc);

    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly ICurrentWorkspace _currentWorkspace;

    public AgentPresencePersistenceTests()
    {
        _presenceRepository = GetRequiredService<IRepository<AgentPresence, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    private Task InsertAsync(Guid tenantId, Guid userId, Guid deviceId, string status = "tracking")
    {
        return WithUnitOfWorkAsync(() => _presenceRepository.InsertAsync(
            new AgentPresence(Guid.NewGuid(), tenantId, userId, deviceId, status, Now),
            autoSave: true));
    }

    [Fact]
    public async Task Should_Persist_And_Update_A_Devices_Presence()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var deviceId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var id = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(() => _presenceRepository.InsertAsync(
                new AgentPresence(id, tenantId, userId, deviceId, "tracking", Now)
                {
                    DeviceName = "DESKTOP-42",
                    Platform = "windows",
                    OsVersion = "Windows 11 Pro 23H2 (10.0.22631)",
                    AgentVersion = "0.2.0",
                    ProjectId = projectId,
                    StatusSince = Now.AddMinutes(-20),
                    CurrentApp = "Visual Studio Code",
                    TimeZone = "Asia/Dhaka",
                    UtcOffsetMinutes = 360
                }));

            var stored = await WithUnitOfWorkAsync(() => _presenceRepository.GetAsync(id));
            stored.WorkspaceId.ShouldBe(tenantId);
            stored.UserId.ShouldBe(userId);
            stored.DeviceId.ShouldBe(deviceId);
            stored.DeviceName.ShouldBe("DESKTOP-42");
            stored.Platform.ShouldBe("windows");
            stored.OsVersion.ShouldBe("Windows 11 Pro 23H2 (10.0.22631)");
            stored.AgentVersion.ShouldBe("0.2.0");
            stored.Status.ShouldBe("tracking");
            stored.ProjectId.ShouldBe(projectId);
            stored.StatusSince.ShouldBe(Now.AddMinutes(-20));
            stored.LastSeenAt.ShouldBe(Now);
            stored.CurrentApp.ShouldBe("Visual Studio Code");
            stored.TimeZone.ShouldBe("Asia/Dhaka");
            stored.UtcOffsetMinutes.ShouldBe(360);

            // A heartbeat updates the same row in place.
            await WithUnitOfWorkAsync(async () =>
            {
                var presence = await _presenceRepository.GetAsync(id);
                presence.Status = "locked";
                presence.StatusSince = Now.AddMinutes(1);
                presence.LastSeenAt = Now.AddMinutes(1);
                presence.CurrentApp = null;
                await _presenceRepository.UpdateAsync(presence);
            });

            var updated = await WithUnitOfWorkAsync(() => _presenceRepository.GetAsync(id));
            updated.Status.ShouldBe("locked");
            updated.LastSeenAt.ShouldBe(Now.AddMinutes(1));
            updated.CurrentApp.ShouldBeNull();
            (await WithUnitOfWorkAsync(() => _presenceRepository.GetCountAsync())).ShouldBe(1);
        }
    }

    [Fact]
    public async Task A_Device_Should_Have_A_Single_Presence_Row_Per_User()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var deviceId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await InsertAsync(tenantId, userId, deviceId);

            await Should.ThrowAsync<DbUpdateException>(() => InsertAsync(tenantId, userId, deviceId, "paused"));

            // The same user on another device, and another user on the same (shared) device, are separate rows.
            await InsertAsync(tenantId, userId, Guid.NewGuid());
            await InsertAsync(tenantId, Guid.NewGuid(), deviceId);

            (await WithUnitOfWorkAsync(() => _presenceRepository.GetCountAsync())).ShouldBe(3);
        }
    }

    [Fact]
    public async Task Presence_Should_Be_Isolated_Per_Workspace()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();
        var deviceId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantA))
        {
            await InsertAsync(tenantA, userId, deviceId);
        }

        using (_currentWorkspace.Change(tenantB))
        {
            (await WithUnitOfWorkAsync(() => _presenceRepository.GetCountAsync())).ShouldBe(0);
        }
    }
}
