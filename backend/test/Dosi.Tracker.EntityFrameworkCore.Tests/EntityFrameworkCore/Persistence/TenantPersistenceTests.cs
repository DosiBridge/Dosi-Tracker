using System;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp.TenantManagement;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

/* Workspace names (AbpTenants.NormalizedName) are unique among live workspaces: the database backstop for two
 * sign-ups of the same name that both pass ABP's name check before either is saved (AgentDataV2Followup). */
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class TenantPersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;

    public TenantPersistenceTests()
    {
        _tenantManager = GetRequiredService<ITenantManager>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
    }

    [Fact]
    public async Task Two_Live_Workspaces_Cannot_Share_A_Name()
    {
        // Both pass ABP's name check, as two concurrent sign-ups do, because neither is saved yet.
        var first = await WithUnitOfWorkAsync(() => _tenantManager.CreateAsync("acme"));
        var second = await WithUnitOfWorkAsync(() => _tenantManager.CreateAsync("acme"));

        await WithUnitOfWorkAsync(() => _tenantRepository.InsertAsync(first, autoSave: true));
        await Should.ThrowAsync<DbUpdateException>(
            () => WithUnitOfWorkAsync(() => _tenantRepository.InsertAsync(second, autoSave: true)));

        var tenants = await WithUnitOfWorkAsync(() => _tenantRepository.GetListAsync());
        tenants.Where(t => t.NormalizedName == "ACME").ShouldHaveSingleItem().Id.ShouldBe(first.Id);
    }

    [Fact]
    public async Task The_Name_Of_A_Deleted_Workspace_Can_Be_Used_Again()
    {
        var deleted = await WithUnitOfWorkAsync(async () =>
            await _tenantRepository.InsertAsync(await _tenantManager.CreateAsync("reborn"), autoSave: true));
        await WithUnitOfWorkAsync(() => _tenantRepository.DeleteAsync(deleted.Id)); // soft delete: the row stays

        var again = await WithUnitOfWorkAsync(async () =>
            await _tenantRepository.InsertAsync(await _tenantManager.CreateAsync("reborn"), autoSave: true));

        again.Id.ShouldNotBe(deleted.Id);
        (await WithUnitOfWorkAsync(() => _tenantRepository.FindByNameAsync("REBORN")))!.Id.ShouldBe(again.Id);
    }
}
