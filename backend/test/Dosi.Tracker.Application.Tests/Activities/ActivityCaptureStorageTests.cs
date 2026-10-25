using System;
using System.IO;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.BlobStoring;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.Activities;

/// <summary>
/// A block and its captures are stored all-or-nothing. The blob store here is <see cref="ScriptedBlobProvider"/>,
/// which can fail an upload or let a concurrent request commit the same block in the middle of one. The blocks
/// belong to a workspace: the unique (TenantId, UserId, ClientActivityId) index that catches duplicate uploads
/// treats a missing tenant (host users) as distinct.
/// </summary>
public abstract class ActivityCaptureStorageTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");
    private static readonly DateTime BaseDay = DateTime.UtcNow.Date.AddDays(-10);

    // Initialized before the base constructor builds the application, so AfterAddApplication can register it.
    private readonly ScriptedBlobProvider _blobProvider = new();

    private readonly IActivityAppService _activityAppService;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    protected ActivityCaptureStorageTests()
    {
        _activityAppService = GetRequiredService<IActivityAppService>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _screenshotRepository = GetRequiredService<IRepository<Screenshot, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private async Task<Guid> SeedProjectAsync(Guid tenantId)
    {
        var projectId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, tenantId, "Tenant Project"));
            await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                new ProjectMember(_guidGenerator.Create(), tenantId, projectId, CurrentUserId));
        });
        return projectId;
    }

    private static CreateActivityDto NewInputWithCaptures(Guid projectId)
    {
        var start = BaseDay.AddHours(10);
        var jpeg = Convert.ToBase64String(new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02, 0xFF, 0xD9 });
        return new CreateActivityDto
        {
            ProjectId = projectId,
            ClientActivityId = Guid.NewGuid(),
            DeviceId = Guid.Parse("5d0c1f6a-3b8e-4a51-9c2e-7f40a1b2c3d4"),
            StartedAt = start,
            EndedAt = start.AddMinutes(10),
            Productivity = 70,
            ScreenshotPngBase64 = jpeg,
            ScreenshotThumbJpgBase64 = jpeg
        };
    }

    private Task<int> CountActivitiesAsync(Guid clientActivityId) =>
        WithUnitOfWorkAsync(() => _activityRepository.CountAsync(a => a.ClientActivityId == clientActivityId));

    private Task<long> CountScreenshotsAsync() =>
        WithUnitOfWorkAsync(() => _screenshotRepository.GetCountAsync());

    [Fact]
    public async Task A_Failed_Capture_Upload_Fails_The_Request_And_Keeps_Nothing_Of_The_Block()
    {
        var tenantId = Guid.NewGuid();
        using (_currentWorkspace.Change(tenantId))
        {
            var input = NewInputWithCaptures(await SeedProjectAsync(tenantId));
            // The screen capture uploads; the thumbnail hits an object-store outage.
            _blobProvider.BeforeSave = (saveNumber, _) =>
                saveNumber == 2 ? throw new IOException("The object store is unreachable.") : Task.CompletedTask;

            // Not answered as success: the agent keeps the block queued and retries it.
            await Should.ThrowAsync<IOException>(() => _activityAppService.CreateAsync(input));

            (await CountActivitiesAsync(input.ClientActivityId)).ShouldBe(0);
            (await CountScreenshotsAsync()).ShouldBe(0);
            _blobProvider.Blobs.ShouldBeEmpty(); // the screen capture already uploaded was removed again

            _blobProvider.BeforeSave = null;
            var retried = await _activityAppService.CreateAsync(input);

            retried.Captures.Count.ShouldBe(2);
            (await CountActivitiesAsync(input.ClientActivityId)).ShouldBe(1);
            _blobProvider.Blobs.Count.ShouldBe(2);
        }
    }

    [Fact]
    public async Task A_Concurrent_Upload_Of_The_Same_Block_Is_Answered_With_The_Block_It_Stored()
    {
        var tenantId = Guid.NewGuid();
        using (_currentWorkspace.Change(tenantId))
        {
            var projectId = await SeedProjectAsync(tenantId);
            var input = NewInputWithCaptures(projectId);
            var winnerId = _guidGenerator.Create();

            // While this upload stores its captures, a retry of the same block handled elsewhere commits it.
            _blobProvider.BeforeSave = async (saveNumber, _) =>
            {
                if (saveNumber == 1)
                {
                    using var uow = GetRequiredService<IUnitOfWorkManager>().Begin(requiresNew: true);
                    await _activityRepository.InsertAsync(new Activity(
                        winnerId, tenantId, CurrentUserId, projectId, input.ClientActivityId, input.StartedAt, input.EndedAt));
                    await uow.CompleteAsync();
                }
            };

            var result = await _activityAppService.CreateAsync(input);

            // Idempotent: the stored block is the answer, and nothing of the losing attempt is left behind.
            result.Id.ShouldBe(winnerId);
            (await CountActivitiesAsync(input.ClientActivityId)).ShouldBe(1);
            (await CountScreenshotsAsync()).ShouldBe(0);
            _blobProvider.Blobs.ShouldBeEmpty();
        }
    }

    [Fact]
    public async Task Captures_Are_Stored_Under_The_Block_With_Their_Detected_Format()
    {
        var tenantId = Guid.NewGuid();
        using (_currentWorkspace.Change(tenantId))
        {
            var input = NewInputWithCaptures(await SeedProjectAsync(tenantId));

            var created = await _activityAppService.CreateAsync(input);

            _blobProvider.Blobs.Keys.ShouldBe(
                new[] { $"activities/{created.Id}/screen.jpg", $"activities/{created.Id}/thumb.jpg" },
                ignoreOrder: true);
            (await CountScreenshotsAsync()).ShouldBe(2);
        }
    }
}
