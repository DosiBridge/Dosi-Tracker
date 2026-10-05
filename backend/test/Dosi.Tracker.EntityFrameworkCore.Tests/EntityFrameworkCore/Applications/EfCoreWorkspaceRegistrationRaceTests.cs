using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.EntityFrameworkCore;
using Volo.Abp.TenantManagement;
using Volo.Abp.TenantManagement.EntityFrameworkCore;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>Two sign-ups pick the same workspace name at the same time: the one whose insert the unique index on the
/// normalized name rejects must get the "name already in use" error, not a server error.</summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreWorkspaceRegistrationRaceTests : TrackerEntityFrameworkCoreTestBase
{
    private const string ContestedName = "contested";

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
        services.Replace(ServiceDescriptor.Transient<ITenantRepository, RaceLosingTenantRepository>());
    }

    [Fact]
    public async Task The_Sign_Up_That_Loses_The_Race_For_A_Name_Should_Be_Told_The_Name_Is_Taken()
    {
        var exception = await Should.ThrowAsync<BusinessException>(
            () => GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
            {
                Name = ContestedName,
                AdminEmail = "late@contested.test",
                AdminPassword = "1q2w3E*late"
            }));

        exception.Code.ShouldBe(WorkspaceAppService.DuplicateTenantNameErrorCode);
        exception.Data["Name"].ShouldBe(ContestedName.ToUpperInvariant());

        // Only the winner's workspace exists.
        var tenants = await WithUnitOfWorkAsync(() => GetRequiredService<ITenantRepository>().GetListAsync());
        tenants.Count(t => t.NormalizedName == ContestedName.ToUpperInvariant()).ShouldBe(1);
    }

    [Fact]
    public async Task Other_Names_Should_Sign_Up_Normally()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "uncontested",
            AdminEmail = "owner@uncontested.test",
            AdminPassword = "1q2w3E*owner"
        });

        result.Name.ShouldBe("uncontested");
    }

    /// <summary>Lets a concurrent sign-up commit the contested name first, then fails the insert the way the unique
    /// index on the normalized name does.</summary>
    public class RaceLosingTenantRepository : EfCoreTenantRepository
    {
        public RaceLosingTenantRepository(IDbContextProvider<ITenantManagementDbContext> dbContextProvider)
            : base(dbContextProvider)
        {
        }

        public override async Task<Tenant> InsertAsync(Tenant entity, bool autoSave = false, CancellationToken cancellationToken = default)
        {
            if (entity.Name != ContestedName)
            {
                return await base.InsertAsync(entity, autoSave, cancellationToken);
            }

            using (var uow = UnitOfWorkManager.Begin(requiresNew: true))
            {
                var winner = await LazyServiceProvider.LazyGetRequiredService<ITenantManager>().CreateAsync(ContestedName);
                await base.InsertAsync(winner, autoSave: true, cancellationToken);
                await uow.CompleteAsync(cancellationToken);
            }

            throw new InvalidOperationException(
                "23505: duplicate key value violates unique constraint \"IX_AbpTenants_NormalizedName\"");
        }
    }
}
