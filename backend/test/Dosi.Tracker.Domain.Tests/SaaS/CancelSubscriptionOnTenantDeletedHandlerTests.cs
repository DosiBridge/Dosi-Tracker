using System;
using System.Threading.Tasks;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Xunit;

namespace Dosi.Tracker.SaaS;

public abstract class CancelSubscriptionOnTenantDeletedHandlerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly ITenantManager _tenantManager;
    private readonly ITenantRepository _tenantRepository;
    private readonly IRepository<Subscription, Guid> _subscriptions;
    private readonly IRepository<Plan, Guid> _plans;
    private readonly ICurrentTenant _currentTenant;
    private readonly IDataFilter _dataFilter;
    private readonly IGuidGenerator _guidGenerator;

    protected CancelSubscriptionOnTenantDeletedHandlerTests()
    {
        _tenantManager = GetRequiredService<ITenantManager>();
        _tenantRepository = GetRequiredService<ITenantRepository>();
        _subscriptions = GetRequiredService<IRepository<Subscription, Guid>>();
        _plans = GetRequiredService<IRepository<Plan, Guid>>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
        _dataFilter = GetRequiredService<IDataFilter>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private async Task<Guid> CreateActiveWorkspaceAsync(string name)
    {
        var starter = await WithUnitOfWorkAsync(() => _plans.FirstAsync(p => p.Name == "Starter"));
        return await WithUnitOfWorkAsync(async () =>
        {
            var tenant = await _tenantManager.CreateAsync(name);
            await _tenantRepository.InsertAsync(tenant);
            using (_currentTenant.Change(tenant.Id))
            {
                await _subscriptions.InsertAsync(new Subscription(_guidGenerator.Create(), tenant.Id, starter.Id)
                {
                    Status = Subscription.ActiveStatus
                });
            }

            return tenant.Id;
        });
    }

    /// <summary>The workspace's subscription, including a soft-deleted one.</summary>
    private async Task<Subscription> GetAnySubscriptionAsync(Guid tenantId)
    {
        using (_currentTenant.Change(tenantId))
        using (_dataFilter.Disable<ISoftDelete>())
        {
            return await WithUnitOfWorkAsync(() => _subscriptions.FirstAsync(s => s.TenantId == tenantId));
        }
    }

    [Fact]
    public async Task Deleting_A_Workspace_Should_Cancel_And_Remove_Its_Subscription()
    {
        var deleted = await CreateActiveWorkspaceAsync("gone");
        var kept = await CreateActiveWorkspaceAsync("kept");

        await WithUnitOfWorkAsync(() => _tenantRepository.DeleteAsync(deleted));

        var canceled = await GetAnySubscriptionAsync(deleted);
        canceled.Status.ShouldBe(Subscription.CanceledStatus);
        canceled.IsDeleted.ShouldBeTrue();
        using (_currentTenant.Change(deleted))
        {
            (await WithUnitOfWorkAsync(() => _subscriptions.AnyAsync(s => s.TenantId == deleted))).ShouldBeFalse();
        }

        var other = await GetAnySubscriptionAsync(kept);
        other.Status.ShouldBe(Subscription.ActiveStatus);
        other.IsDeleted.ShouldBeFalse();
    }
}
