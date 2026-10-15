using System;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

/* The workspace model's database guarantees: unique names and memberships, and the workspace data filter that keeps
 * every workspace's rows to itself. */
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class WorkspacePersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspacePersistenceTests()
    {
        _workspaceRepository = GetRequiredService<IRepository<Workspace, Guid>>();
        _memberRepository = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _activityRepository = GetRequiredService<IRepository<Activity, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    [Fact]
    public async Task Two_Workspaces_Cannot_Share_A_Name_Whatever_Its_Case()
    {
        await WithUnitOfWorkAsync(() => _workspaceRepository.InsertAsync(new Workspace(Guid.NewGuid(), "Acme"), autoSave: true));

        await Should.ThrowAsync<DbUpdateException>(() =>
            WithUnitOfWorkAsync(() => _workspaceRepository.InsertAsync(new Workspace(Guid.NewGuid(), " ACME "), autoSave: true)));
    }

    [Fact]
    public async Task A_Person_Has_One_Membership_Per_Workspace()
    {
        var userId = Guid.NewGuid();
        await WithUnitOfWorkAsync(() => _memberRepository.InsertAsync(
            new WorkspaceMember(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, userId, WorkspaceRoles.Member, DateTime.UtcNow), autoSave: true));

        await Should.ThrowAsync<DbUpdateException>(() => WithUnitOfWorkAsync(() => _memberRepository.InsertAsync(
            new WorkspaceMember(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, userId, WorkspaceRoles.Owner, DateTime.UtcNow), autoSave: true)));
    }

    [Fact]
    public async Task Each_Workspace_Should_See_Only_Its_Own_Rows_Unless_The_Filter_Is_Disabled()
    {
        var a = await ServiceProvider.CreateWorkspaceAsync("isolation-a");
        var b = await ServiceProvider.CreateWorkspaceAsync("isolation-b");
        using (_currentWorkspace.Change(a))
        {
            await WithUnitOfWorkAsync(() => _projectRepository.InsertAsync(new Project(Guid.NewGuid(), a, "A only")));
        }

        using (_currentWorkspace.Change(b))
        {
            await WithUnitOfWorkAsync(() => _projectRepository.InsertAsync(new Project(Guid.NewGuid(), b, "B only")));
            (await WithUnitOfWorkAsync(() => _projectRepository.GetListAsync())).ShouldHaveSingleItem().Title.ShouldBe("B only");
        }

        using (_currentWorkspace.Change(a))
        {
            (await WithUnitOfWorkAsync(() => _projectRepository.GetListAsync())).ShouldHaveSingleItem().Title.ShouldBe("A only");
        }

        using (_currentWorkspace.Change(null))
        {
            (await WithUnitOfWorkAsync(() => _projectRepository.GetListAsync())).ShouldBeEmpty();

            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                (await WithUnitOfWorkAsync(() => _projectRepository.GetListAsync())).Count.ShouldBe(2);
            }
        }
    }

    [Fact]
    public async Task A_New_Row_Should_Get_The_Current_Workspace_And_Need_One()
    {
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("autoset");
        var started = DateTime.UtcNow.AddHours(-1);

        using (_currentWorkspace.Change(workspaceId))
        {
            var activity = await WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(
                new Activity(Guid.NewGuid(), Guid.Empty, Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), started, started.AddMinutes(10)),
                autoSave: true));
            activity.WorkspaceId.ShouldBe(workspaceId);
        }

        using (_currentWorkspace.Change(null))
        {
            (await Should.ThrowAsync<BusinessException>(() => WithUnitOfWorkAsync(() => _activityRepository.InsertAsync(
                    new Activity(Guid.NewGuid(), Guid.Empty, Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), started, started.AddMinutes(10)),
                    autoSave: true))))
                .Code.ShouldBe(TrackerDomainErrorCodes.WorkspaceRequired);
        }
    }
}
