using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Repositories.EntityFrameworkCore;
using Volo.Abp.EntityFrameworkCore;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>Two sign-ups pick the same workspace name at the same time: the one whose insert the unique index on the
/// normalized name rejects must get the "name already in use" error, not a server error, and leave nothing behind.</summary>
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class EfCoreWorkspaceRegistrationRaceTests : TrackerEntityFrameworkCoreTestBase
{
    private const string ContestedName = "contested";

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
        services.Replace(ServiceDescriptor.Transient<IRepository<Workspace, Guid>, RaceLosingWorkspaceRepository>());
    }

    [Fact]
    public async Task The_Sign_Up_That_Loses_The_Race_For_A_Name_Should_Be_Told_The_Name_Is_Taken()
    {
        var exception = await Should.ThrowAsync<BusinessException>(
            () => GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
            {
                WorkspaceName = ContestedName,
                Email = "late@contested.test",
                Password = "1q2w3E*late"
            }));

        exception.Code.ShouldBe(TrackerDomainErrorCodes.DuplicateWorkspaceName);

        var workspaces = await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Workspace, Guid>>().GetListAsync());
        workspaces.Count(w => w.NormalizedName == ContestedName.ToUpperInvariant()).ShouldBe(1);
    }

    [Fact]
    public async Task Other_Names_Should_Sign_Up_Normally()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            WorkspaceName = "uncontested",
            Email = "owner@uncontested.test",
            Password = "1q2w3E*owner"
        });

        result.WorkspaceName.ShouldBe("uncontested");
    }

    /// <summary>Lets a concurrent sign-up commit the contested name first, then fails the insert the way the unique
    /// index on the normalized name does.</summary>
    public class RaceLosingWorkspaceRepository : EfCoreRepository<TrackerDbContext, Workspace, Guid>
    {
        public RaceLosingWorkspaceRepository(IDbContextProvider<TrackerDbContext> dbContextProvider)
            : base(dbContextProvider)
        {
        }

        public override async Task<Workspace> InsertAsync(Workspace entity, bool autoSave = false, CancellationToken cancellationToken = default)
        {
            if (entity.Name != ContestedName)
            {
                return await base.InsertAsync(entity, autoSave, cancellationToken);
            }

            using (var uow = UnitOfWorkManager.Begin(requiresNew: true))
            {
                await base.InsertAsync(new Workspace(Guid.NewGuid(), ContestedName), autoSave: true, cancellationToken);
                await uow.CompleteAsync(cancellationToken);
            }

            throw new InvalidOperationException(
                "23505: duplicate key value violates unique constraint \"IX_AppWorkspaces_NormalizedName\"");
        }
    }
}
