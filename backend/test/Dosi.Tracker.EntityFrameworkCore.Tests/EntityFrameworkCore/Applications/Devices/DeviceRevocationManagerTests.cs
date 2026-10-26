using System;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>Which device an upload or heartbeat is recorded for, and which are refused
/// (<see cref="DeviceRevocationManager.EnsureCanIngestAsync"/>).</summary>
public class DeviceRevocationManagerTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly Guid WorkspaceId = Guid.Parse("ad701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid UserId = Guid.Parse("ad701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Revoked = Guid.Parse("ad701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid Fresh = Guid.Parse("ad701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid Other = Guid.Parse("ad701e62-0953-4dd3-910b-dc6cc93ccb05");

    private Task SeedRevocationAsync()
    {
        return WithUnitOfWorkAsync(() => GetRequiredService<IRepository<DeviceRevocation, Guid>>().InsertAsync(
            new DeviceRevocation(Guid.NewGuid(), WorkspaceId, UserId, Revoked, DateTime.UtcNow), autoSave: true));
    }

    private Task<Guid> IngestAsync(Guid? token, Guid? request, Guid? workspaceId = null)
    {
        var workspace = workspaceId ?? WorkspaceId;
        return WithUnitOfWorkAsync(async () =>
        {
            // As in a request: the workspace it is made in is the current one.
            using (GetRequiredService<ICurrentWorkspace>().Change(workspace))
            {
                return await GetRequiredService<DeviceRevocationManager>().EnsureCanIngestAsync(workspace, UserId, token, request);
            }
        });
    }

    [Theory]
    [InlineData(true, false)]  // revoked token, fresh body id: the bypass a regenerated id used to be
    [InlineData(false, true)]  // fresh token, revoked body id (a queued row of the revoked install)
    [InlineData(true, true)]
    public async Task A_Revoked_Device_Is_Refused_Whether_Named_By_The_Token_Or_The_Body(bool tokenRevoked, bool bodyRevoked)
    {
        await SeedRevocationAsync();

        var exception = await Should.ThrowAsync<BusinessException>(() =>
            IngestAsync(tokenRevoked ? Revoked : Fresh, bodyRevoked ? Revoked : Other));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.DeviceRevoked);
        exception.Data["deviceId"].ShouldBe(Revoked);
    }

    [Fact]
    public async Task The_Body_Device_Is_Recorded_Else_The_Tokens()
    {
        await SeedRevocationAsync();

        (await IngestAsync(Fresh, Other)).ShouldBe(Other);
        (await IngestAsync(Fresh, null)).ShouldBe(Fresh);
        (await IngestAsync(Fresh, Guid.Empty)).ShouldBe(Fresh);
        (await IngestAsync(null, Other)).ShouldBe(Other);
        (await IngestAsync(Guid.Empty, Other)).ShouldBe(Other);

        // The revocation is the workspace's: elsewhere the same install is accepted.
        (await IngestAsync(Revoked, null, Guid.NewGuid())).ShouldBe(Revoked);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task A_Request_Naming_No_Device_Is_Refused(bool emptyIds)
    {
        var exception = await Should.ThrowAsync<BusinessException>(() =>
            IngestAsync(emptyIds ? Guid.Empty : null, emptyIds ? Guid.Empty : null));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.DeviceIdRequired);
    }
}
