using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Reporting;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class ProjectMemberRatePersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly DateTime October = new(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc);

    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly ICurrentWorkspace _currentWorkspace;

    public ProjectMemberRatePersistenceTests()
    {
        _rateRepository = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    [Fact]
    public async Task Should_Persist_A_Members_Rate_History()
    {
        var tenantId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _rateRepository.InsertAsync(new ProjectMemberRate(
                    Guid.NewGuid(), tenantId, projectId, userId, 20m, new DateTime(2000, 1, 1, 0, 0, 0, DateTimeKind.Utc)));
                await _rateRepository.InsertAsync(new ProjectMemberRate(
                    Guid.NewGuid(), tenantId, projectId, userId, 30.25m, October));
            });

            var history = await WithUnitOfWorkAsync(async () =>
                (await _rateRepository.GetListAsync(r => r.ProjectId == projectId && r.UserId == userId))
                    .OrderBy(r => r.EffectiveFrom)
                    .ToList());

            history.Count.ShouldBe(2);
            history[0].HourlyRate.ShouldBe(20m);
            history[1].HourlyRate.ShouldBe(30.25m);
            history[1].EffectiveFrom.ShouldBe(October);
            history.ShouldAllBe(r => r.WorkspaceId == tenantId && r.CreationTime != default(DateTime));
        }
    }

    [Fact]
    public async Task The_Rate_In_Effect_Should_Be_The_Latest_One_That_Started_Before_The_Moment()
    {
        // Payroll's lookup (HourlyRateSchedule, as GetPayrollAsync builds it) over the history as stored: the latest
        // EffectiveFrom <= the block's start; a block from before the history is priced at the first rate.
        var tenantId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                // Recorded out of order, and with two entries for the same moment (the one recorded last wins).
                await _rateRepository.InsertAsync(new ProjectMemberRate(Guid.NewGuid(), tenantId, projectId, userId, 30m, October));
                await _rateRepository.InsertAsync(new ProjectMemberRate(Guid.NewGuid(), tenantId, projectId, userId, 20m, October.AddMonths(-1)));
            });
            await WithUnitOfWorkAsync(() => _rateRepository.InsertAsync(
                new ProjectMemberRate(Guid.NewGuid(), tenantId, projectId, userId, 35m, October.AddDays(15))));
            await WithUnitOfWorkAsync(() => _rateRepository.InsertAsync(
                new ProjectMemberRate(Guid.NewGuid(), tenantId, projectId, userId, 36m, October.AddDays(15))));

            var schedule = HourlyRateSchedule.From(await WithUnitOfWorkAsync(() =>
                _rateRepository.GetListAsync(r => r.ProjectId == projectId && r.UserId == userId)));

            schedule.RateAt(October.AddDays(-1)).ShouldBe(20m); // September work keeps the September rate
            schedule.RateAt(October).ShouldBe(30m);
            schedule.RateAt(October.AddDays(14)).ShouldBe(30m);
            schedule.RateAt(October.AddDays(15)).ShouldBe(36m);
            schedule.RateAt(October.AddMonths(-2)).ShouldBe(20m); // before the history: the first rate the member was given
        }
    }

    [Fact]
    public async Task Rates_Should_Be_Isolated_Per_Workspace()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();

        using (_currentWorkspace.Change(tenantA))
        {
            await WithUnitOfWorkAsync(() => _rateRepository.InsertAsync(
                new ProjectMemberRate(Guid.NewGuid(), tenantA, Guid.NewGuid(), Guid.NewGuid(), 15m, October)));
        }

        using (_currentWorkspace.Change(tenantB))
        {
            (await WithUnitOfWorkAsync(() => _rateRepository.GetCountAsync())).ShouldBe(0);
        }

        using (_currentWorkspace.Change(tenantA))
        {
            (await WithUnitOfWorkAsync(() => _rateRepository.GetCountAsync())).ShouldBe(1);
        }
    }
}
