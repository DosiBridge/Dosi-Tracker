using System;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Volo.Abp.Uow;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker;

/// <summary>Outside any explicit change, tests work in <see cref="TrackerTestConsts.WorkspaceId"/>.</summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(ICurrentWorkspace), typeof(CurrentWorkspace))]
public class TestCurrentWorkspace : CurrentWorkspace
{
    protected override Guid? DefaultId => TrackerTestConsts.WorkspaceId;
}

public class TestWorkspaceDataSeedContributor : IDataSeedContributor, ITransientDependency
{
    private readonly IRepository<Workspace, Guid> _workspaceRepository;

    public TestWorkspaceDataSeedContributor(IRepository<Workspace, Guid> workspaceRepository)
    {
        _workspaceRepository = workspaceRepository;
    }

    public async Task SeedAsync(DataSeedContext context)
    {
        if (await _workspaceRepository.FindAsync(TrackerTestConsts.WorkspaceId) == null)
        {
            await _workspaceRepository.InsertAsync(new Workspace(TrackerTestConsts.WorkspaceId, TrackerTestConsts.WorkspaceName), autoSave: true);
        }
    }
}

/// <summary>Creates workspaces and memberships for tests, each in a unit of work of its own.</summary>
public static class WorkspaceTestData
{
    public static async Task<Guid> CreateWorkspaceAsync(this IServiceProvider serviceProvider, string name, Guid? id = null)
    {
        var workspaceId = id ?? Guid.NewGuid();
        using var scope = serviceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true);
        await scope.ServiceProvider.GetRequiredService<IRepository<Workspace, Guid>>()
            .InsertAsync(new Workspace(workspaceId, name), autoSave: true);
        await uow.CompleteAsync();
        return workspaceId;
    }

    /// <summary>Makes the user an active member of the workspace with the role (default: the test workspace).</summary>
    public static async Task AddWorkspaceMemberAsync(this IServiceProvider serviceProvider, Guid userId, string role, Guid? workspaceId = null)
    {
        using var scope = serviceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true);
        var id = workspaceId ?? TrackerTestConsts.WorkspaceId;
        var workspaces = scope.ServiceProvider.GetRequiredService<IRepository<Workspace, Guid>>();
        if (await workspaces.FindAsync(id) == null)
        {
            await workspaces.InsertAsync(new Workspace(id, "Workspace " + id.ToString("N")), autoSave: true);
        }

        await scope.ServiceProvider.GetRequiredService<WorkspaceMembershipManager>().AddAsync(id, userId, role);
        await uow.CompleteAsync();
    }

    public static async Task<WorkspaceMember?> FindWorkspaceMemberAsync(this IServiceProvider serviceProvider, Guid userId, Guid? workspaceId = null)
    {
        using var scope = serviceProvider.CreateScope();
        var uowManager = scope.ServiceProvider.GetRequiredService<IUnitOfWorkManager>();
        using var uow = uowManager.Begin(requiresNew: true);
        var id = workspaceId ?? TrackerTestConsts.WorkspaceId;
        var member = await scope.ServiceProvider.GetRequiredService<IRepository<WorkspaceMember, Guid>>()
            .FirstOrDefaultAsync(m => m.WorkspaceId == id && m.UserId == userId);
        await uow.CompleteAsync();
        return member;
    }
}
