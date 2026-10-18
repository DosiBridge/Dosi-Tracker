using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Validation;
using Xunit;

namespace Dosi.Tracker.Presence;

/// <summary>Heartbeats and the team's live presence. The test caller holds every permission, Activities.ViewAll included.</summary>
public abstract class PresenceAppServiceTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    private readonly IPresenceAppService _presenceAppService;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected PresenceAppServiceTests()
    {
        _presenceAppService = GetRequiredService<IPresenceAppService>();
        _presenceRepository = GetRequiredService<IRepository<AgentPresence, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    private async Task<Guid> SeedProjectAsync(string title = "Website redesign", Action<Project>? configure = null)
    {
        var project = new Project(_guidGenerator.Create(), TrackerTestConsts.WorkspaceId, title);
        configure?.Invoke(project);
        await WithUnitOfWorkAsync(() => _projectRepository.InsertAsync(project));
        return project.Id;
    }

    private Task<IdentityUser> CreateUserAsync(Guid id, string userName, string? name = null, string? surname = null, Guid? tenantId = null)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(id, userName, $"{userName}@acme.test") { Name = name, Surname = surname };
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
            return user;
        });
    }

    private Task InsertPresenceAsync(Guid userId, Guid deviceId, DateTime lastSeenAt, Guid? tenantId = null, string? deviceName = null)
    {
        return WithUnitOfWorkAsync(() => _presenceRepository.InsertAsync(
            new AgentPresence(_guidGenerator.Create(), tenantId ?? TrackerTestConsts.WorkspaceId, userId, deviceId, "tracking", lastSeenAt) { DeviceName = deviceName }));
    }

    private Task<AgentPresence> GetDeviceAsync(Guid deviceId) =>
        WithUnitOfWorkAsync(() => _presenceRepository.GetAsync(p => p.DeviceId == deviceId));

    private Task<int> CountDeviceRowsAsync(Guid deviceId) =>
        WithUnitOfWorkAsync(() => _presenceRepository.CountAsync(p => p.DeviceId == deviceId));

    // ---- Heartbeat ----

    [Fact]
    public async Task Heartbeat_Should_Record_The_State_Of_A_Device()
    {
        var projectId = await SeedProjectAsync();
        var deviceId = Guid.NewGuid();
        var before = DateTime.UtcNow;

        var result = await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId,
            DeviceName = " DESKTOP-42 ",
            Platform = "Windows",
            OsVersion = "Windows 11 Pro 23H2 (10.0.22631)",
            AgentVersion = "0.2.0",
            Status = "tracking",
            ProjectId = projectId,
            CurrentApp = "Visual Studio Code",
            TimeZone = "Asia/Dhaka",
            UtcOffsetMinutes = 360
        });

        result.ServerTime.ShouldBeInRange(before.AddSeconds(-1), DateTime.UtcNow.AddSeconds(1));

        var stored = await GetDeviceAsync(deviceId);
        stored.UserId.ShouldBe(CurrentUserId);
        stored.WorkspaceId.ShouldBe(TrackerTestConsts.WorkspaceId);
        stored.DeviceName.ShouldBe("DESKTOP-42");
        stored.Platform.ShouldBe("windows");
        stored.OsVersion.ShouldBe("Windows 11 Pro 23H2 (10.0.22631)");
        stored.AgentVersion.ShouldBe("0.2.0");
        stored.Status.ShouldBe("tracking");
        stored.ProjectId.ShouldBe(projectId);
        stored.CurrentApp.ShouldBe("Visual Studio Code");
        stored.TimeZone.ShouldBe("Asia/Dhaka");
        stored.UtcOffsetMinutes.ShouldBe(360);
        stored.LastSeenAt.ShouldBe(result.ServerTime);
        stored.StatusSince.ShouldBe(result.ServerTime); // the agent did not say: the status starts now
    }

    [Fact]
    public async Task Heartbeat_Should_Update_The_Row_Of_The_Device()
    {
        var projectId = await SeedProjectAsync();
        var deviceId = Guid.NewGuid();
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId,
            DeviceName = "DESKTOP-42",
            Platform = "windows",
            AgentVersion = "0.2.0",
            TimeZone = "Asia/Dhaka",
            Status = "tracking",
            ProjectId = projectId,
            CurrentApp = "Slack"
        });

        // A minimal heartbeat (e.g. on pause): the moment changes, the device stays described.
        var paused = await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = deviceId, Status = "paused" });

        (await CountDeviceRowsAsync(deviceId)).ShouldBe(1);
        var stored = await GetDeviceAsync(deviceId);
        stored.Status.ShouldBe("paused");
        stored.StatusSince.ShouldBe(paused.ServerTime);
        stored.LastSeenAt.ShouldBe(paused.ServerTime);
        stored.ProjectId.ShouldBeNull();
        stored.CurrentApp.ShouldBeNull();
        stored.DeviceName.ShouldBe("DESKTOP-42");
        stored.Platform.ShouldBe("windows");
        stored.AgentVersion.ShouldBe("0.2.0");
        stored.TimeZone.ShouldBe("Asia/Dhaka");

        // Still paused a minute later: the status keeps its start.
        var stillPaused = await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = deviceId, Status = "paused" });

        stored = await GetDeviceAsync(deviceId);
        stored.StatusSince.ShouldBe(paused.ServerTime);
        stored.LastSeenAt.ShouldBe(stillPaused.ServerTime);
    }

    [Fact]
    public async Task Heartbeat_Should_Keep_A_Row_Per_Device()
    {
        var laptop = Guid.NewGuid();
        var desktop = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = laptop, Status = "tracking" });
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = desktop, Status = "idle" });

        (await GetDeviceAsync(laptop)).Status.ShouldBe("tracking");
        (await GetDeviceAsync(desktop)).Status.ShouldBe("idle");
        (await WithUnitOfWorkAsync(() => _presenceRepository.CountAsync(p => p.UserId == CurrentUserId))).ShouldBe(2);
    }

    [Fact]
    public async Task A_New_Device_Should_Drop_The_Least_Recently_Seen_Devices_Beyond_The_Cap()
    {
        // Reinstalls (or a script inventing device ids) left more devices than a member keeps, last seen 1 to 12 hours ago.
        var devices = Enumerable.Range(1, AgentPresenceConsts.MaxDevicesPerUser + 2).Select(_ => Guid.NewGuid()).ToList();
        for (var i = 0; i < devices.Count; i++)
        {
            await InsertPresenceAsync(CurrentUserId, devices[i], DateTime.UtcNow.AddHours(-(i + 1)));
        }

        var newcomer = Guid.NewGuid();
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = newcomer, Status = "tracking" });

        var kept = await WithUnitOfWorkAsync(() => _presenceRepository.GetListAsync(p => p.UserId == CurrentUserId));
        kept.Select(p => p.DeviceId).ShouldBe(
            devices.Take(AgentPresenceConsts.MaxDevicesPerUser - 1).Append(newcomer),
            ignoreOrder: true);
    }

    [Fact]
    public async Task A_New_Device_Should_Drop_The_Members_Devices_Not_Seen_For_A_Month()
    {
        var recent = Guid.NewGuid();
        var stale = Guid.NewGuid();
        var colleaguesStale = Guid.NewGuid();
        await InsertPresenceAsync(CurrentUserId, recent, DateTime.UtcNow.AddDays(-2));
        await InsertPresenceAsync(CurrentUserId, stale, DateTime.UtcNow.AddDays(-(AgentPresenceConsts.StaleDeviceDays + 1)));
        await InsertPresenceAsync(_guidGenerator.Create(), colleaguesStale, DateTime.UtcNow.AddDays(-90));

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = Guid.NewGuid(), Status = "tracking" });

        (await CountDeviceRowsAsync(recent)).ShouldBe(1);
        (await CountDeviceRowsAsync(stale)).ShouldBe(0);
        (await CountDeviceRowsAsync(colleaguesStale)).ShouldBe(1); // another member's devices are theirs to tidy
    }

    [Fact]
    public async Task Heartbeats_Of_A_Known_Device_Should_Leave_The_Other_Devices_Alone()
    {
        var known = Guid.NewGuid();
        var stale = Guid.NewGuid();
        await InsertPresenceAsync(CurrentUserId, known, DateTime.UtcNow.AddMinutes(-1));
        await InsertPresenceAsync(CurrentUserId, stale, DateTime.UtcNow.AddDays(-90));

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = known, Status = "paused" });

        (await CountDeviceRowsAsync(stale)).ShouldBe(1);
    }

    [Theory]
    [InlineData("working")]
    [InlineData("tracking-hard")]
    [InlineData("")]
    public async Task Heartbeat_Should_Reject_A_Status_Outside_The_List(string status)
    {
        var deviceId = Guid.NewGuid();

        await Should.ThrowAsync<AbpValidationException>(() =>
            _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = deviceId, Status = status }));

        (await CountDeviceRowsAsync(deviceId)).ShouldBe(0);
    }

    [Fact]
    public async Task Heartbeat_Should_Accept_Every_Status_Of_The_List_In_Any_Case()
    {
        foreach (var status in AgentPresenceConsts.Statuses)
        {
            var deviceId = Guid.NewGuid();
            await _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = deviceId, Status = " " + status.ToUpperInvariant() });

            (await GetDeviceAsync(deviceId)).Status.ShouldBe(status);
        }
    }

    [Fact]
    public async Task Heartbeat_Should_Require_A_Device_Id()
    {
        await Should.ThrowAsync<AbpValidationException>(() =>
            _presenceAppService.HeartbeatAsync(new HeartbeatDto { DeviceId = Guid.Empty, Status = "tracking" }));
    }

    [Fact]
    public async Task Heartbeat_Should_Drop_The_Current_App_When_The_Project_Does_Not_Allow_The_Active_Window()
    {
        var projectId = await SeedProjectAsync(configure: p => p.AllowActiveWindow = false);
        var deviceId = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId,
            Status = "tracking",
            ProjectId = projectId,
            CurrentApp = "Outlook"
        });

        var stored = await GetDeviceAsync(deviceId);
        stored.ProjectId.ShouldBe(projectId);
        stored.CurrentApp.ShouldBeNull();
    }

    [Fact]
    public async Task Heartbeat_Should_Drop_The_Current_App_Without_A_Known_Project()
    {
        var unknownProject = Guid.NewGuid();
        var withUnknownProject = Guid.NewGuid();
        var withoutProject = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = withUnknownProject, Status = "tracking", ProjectId = unknownProject, CurrentApp = "Outlook"
        });
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = withoutProject, Status = "choosing", CurrentApp = "Outlook"
        });

        var unknown = await GetDeviceAsync(withUnknownProject);
        unknown.ProjectId.ShouldBeNull();
        unknown.CurrentApp.ShouldBeNull();
        (await GetDeviceAsync(withoutProject)).CurrentApp.ShouldBeNull();
    }

    [Fact]
    public async Task Heartbeat_Should_Truncate_A_Long_Current_App()
    {
        var projectId = await SeedProjectAsync();
        var deviceId = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId, Status = "tracking", ProjectId = projectId, CurrentApp = new string('a', 300)
        });

        (await GetDeviceAsync(deviceId)).CurrentApp.ShouldBe(new string('a', AgentPresenceConsts.MaxCurrentAppLength));
    }

    [Fact]
    public async Task Heartbeat_Should_Bring_The_Status_Time_Onto_The_Server_Clock()
    {
        // The device clock is 5 minutes ahead; it entered "idle" 10 minutes ago by its own clock.
        var deviceNow = DateTime.UtcNow.AddMinutes(5);
        var deviceId = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId, Status = "idle", SentAt = deviceNow, StatusSince = deviceNow.AddMinutes(-10)
        });

        var stored = await GetDeviceAsync(deviceId);
        stored.StatusSince!.Value.ShouldBeInRange(DateTime.UtcNow.AddMinutes(-10).AddSeconds(-5), DateTime.UtcNow.AddMinutes(-10).AddSeconds(5));
    }

    [Fact]
    public async Task Heartbeat_Should_Never_Date_A_Status_In_The_Future()
    {
        var deviceId = Guid.NewGuid();

        var result = await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId, Status = "tracking", StatusSince = DateTime.UtcNow.AddHours(2)
        });

        (await GetDeviceAsync(deviceId)).StatusSince.ShouldBe(result.ServerTime);
    }

    [Fact]
    public async Task Heartbeat_Should_Store_Unusable_Context_Values_As_Null()
    {
        var deviceId = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = deviceId,
            Status = "tracking",
            DeviceName = "\u0001\u0002",
            Platform = "freebsd",
            TimeZone = "Nowhere/Atlantis",
            UtcOffsetMinutes = 2000
        });

        var stored = await GetDeviceAsync(deviceId);
        stored.DeviceName.ShouldBeNull();
        stored.Platform.ShouldBeNull();
        stored.TimeZone.ShouldBeNull();
        stored.UtcOffsetMinutes.ShouldBeNull();

        var linuxDeviceId = Guid.NewGuid();
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = linuxDeviceId,
            Status = "tracking",
            Platform = "linux"
        });
        (await GetDeviceAsync(linuxDeviceId)).Platform.ShouldBe("linux");
    }

    // ---- Team ----

    [Fact]
    public async Task GetTeam_Should_List_Every_Member_Device_With_Names_And_Projects()
    {
        var bobId = _guidGenerator.Create();
        await CreateUserAsync(CurrentUserId, "ana", "Ana", "Lima");
        await CreateUserAsync(bobId, "bob");
        var projectId = await SeedProjectAsync("Mobile app");
        var anaDevice = Guid.NewGuid();
        var bobDevice = Guid.NewGuid();

        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = anaDevice, Status = "tracking", ProjectId = projectId, CurrentApp = "Figma", DeviceName = "ANA-MAC", Platform = "macos"
        });
        await InsertPresenceAsync(bobId, bobDevice, DateTime.UtcNow.AddMinutes(-10), deviceName: "BOB-PC");

        var team = (await _presenceAppService.GetTeamAsync()).Items;

        team.Count.ShouldBe(2);
        var ana = team[0]; // online devices first
        ana.UserId.ShouldBe(CurrentUserId);
        ana.UserName.ShouldBe("ana");
        ana.Name.ShouldBe("Ana Lima");
        ana.DeviceId.ShouldBe(anaDevice);
        ana.DeviceName.ShouldBe("ANA-MAC");
        ana.Platform.ShouldBe("macos");
        ana.Status.ShouldBe("tracking");
        ana.ProjectId.ShouldBe(projectId);
        ana.ProjectTitle.ShouldBe("Mobile app");
        ana.CurrentApp.ShouldBe("Figma");
        ana.Online.ShouldBeTrue();

        var bob = team[1];
        bob.UserId.ShouldBe(bobId);
        bob.UserName.ShouldBe("bob");
        bob.Name.ShouldBeNull();
        bob.DeviceName.ShouldBe("BOB-PC");
        bob.ProjectTitle.ShouldBeNull();
        bob.Online.ShouldBeFalse();
    }

    [Fact]
    public async Task GetTeam_Should_Count_A_Device_As_Online_For_150_Seconds_After_Its_Last_Heartbeat()
    {
        await CreateUserAsync(CurrentUserId, "ana");
        var recent = Guid.NewGuid();
        var stale = Guid.NewGuid();
        await InsertPresenceAsync(CurrentUserId, recent, DateTime.UtcNow.AddSeconds(-100));
        await InsertPresenceAsync(CurrentUserId, stale, DateTime.UtcNow.AddSeconds(-AgentPresenceConsts.OnlineWindowSeconds - 30));

        var team = (await _presenceAppService.GetTeamAsync()).Items;

        team.Single(p => p.DeviceId == recent).Online.ShouldBeTrue();
        team.Single(p => p.DeviceId == stale).Online.ShouldBeFalse();
    }

    [Fact]
    public async Task GetTeam_Should_Leave_Out_Devices_Of_Deleted_And_Deactivated_Accounts()
    {
        await CreateUserAsync(CurrentUserId, "ana");
        var deleted = await CreateUserAsync(_guidGenerator.Create(), "gone");
        var deactivated = await CreateUserAsync(_guidGenerator.Create(), "left");
        await InsertPresenceAsync(CurrentUserId, Guid.NewGuid(), DateTime.UtcNow);
        await InsertPresenceAsync(deleted.Id, Guid.NewGuid(), DateTime.UtcNow);
        await InsertPresenceAsync(deactivated.Id, Guid.NewGuid(), DateTime.UtcNow);
        await InsertPresenceAsync(_guidGenerator.Create(), Guid.NewGuid(), DateTime.UtcNow); // no account at all

        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.DeleteAsync(await userManager.GetByIdAsync(deleted.Id))).Succeeded.ShouldBeTrue();
            var leaver = await userManager.GetByIdAsync(deactivated.Id);
            leaver.SetIsActive(false);
            (await userManager.UpdateAsync(leaver)).Succeeded.ShouldBeTrue();
        });

        var team = (await _presenceAppService.GetTeamAsync()).Items;

        team.ShouldHaveSingleItem().UserId.ShouldBe(CurrentUserId);
    }

    [Fact]
    public async Task GetTeam_Should_Only_List_Devices_Of_The_Current_Workspace()
    {
        await CreateUserAsync(CurrentUserId, "ana");
        await InsertPresenceAsync(CurrentUserId, Guid.NewGuid(), DateTime.UtcNow);

        var otherTenantId = Guid.NewGuid();
        using (GetRequiredService<ICurrentWorkspace>().Change(otherTenantId))
        {
            var carol = await CreateUserAsync(_guidGenerator.Create(), "carol", tenantId: otherTenantId);
            await InsertPresenceAsync(carol.Id, Guid.NewGuid(), DateTime.UtcNow, otherTenantId);

            (await _presenceAppService.GetTeamAsync()).Items.ShouldHaveSingleItem().UserName.ShouldBe("carol");
        }

        (await _presenceAppService.GetTeamAsync()).Items.ShouldHaveSingleItem().UserName.ShouldBe("ana");
    }

    [Fact]
    public async Task GetTeam_Should_Hide_The_Current_App_Once_The_Project_Stops_Allowing_The_Active_Window()
    {
        await CreateUserAsync(CurrentUserId, "ana");
        var projectId = await SeedProjectAsync();
        await _presenceAppService.HeartbeatAsync(new HeartbeatDto
        {
            DeviceId = Guid.NewGuid(), Status = "tracking", ProjectId = projectId, CurrentApp = "Figma"
        });

        await WithUnitOfWorkAsync(async () =>
        {
            var project = await _projectRepository.GetAsync(projectId);
            project.AllowActiveWindow = false;
        });

        var device = (await _presenceAppService.GetTeamAsync()).Items.ShouldHaveSingleItem();
        device.ProjectTitle.ShouldBe("Website redesign");
        device.CurrentApp.ShouldBeNull();
    }
}
