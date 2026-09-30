using System.Threading.Tasks;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Seeds the built-in "manager" role. The data seeder runs with the tenant's context for every tenant
/// creation path — workspace sign-up (WorkspaceAppService.RegisterAsync), the host console's ABP tenant
/// endpoint (TenantAppService.CreateAsync) — and for every existing tenant when the DbMigrator runs.
/// </summary>
public class ManagerRoleDataSeedContributor : IDataSeedContributor, ITransientDependency
{
    private readonly ManagerRoleProvisioner _provisioner;
    private readonly ICurrentTenant _currentTenant;

    public ManagerRoleDataSeedContributor(ManagerRoleProvisioner provisioner, ICurrentTenant currentTenant)
    {
        _provisioner = provisioner;
        _currentTenant = currentTenant;
    }

    [UnitOfWork]
    public virtual async Task SeedAsync(DataSeedContext context)
    {
        using (_currentTenant.Change(context.TenantId))
        {
            await _provisioner.EnsureAsync();
        }
    }
}
