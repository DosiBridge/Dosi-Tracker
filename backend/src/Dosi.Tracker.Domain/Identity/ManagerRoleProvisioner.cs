using System;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Domain.Services;
using Volo.Abp.Identity;
using IdentityRole = Volo.Abp.Identity.IdentityRole;
using Volo.Abp.PermissionManagement;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Ensures the current tenant (or the host) has the built-in <see cref="TrackerRoles.Manager"/> role.
/// Its permissions are granted only when the role is first created, so a workspace admin who later
/// trims the role's permissions is not overridden by the next DbMigrator run.
/// </summary>
public class ManagerRoleProvisioner : DomainService
{
    private readonly IdentityRoleManager _roleManager;
    private readonly IPermissionDataSeeder _permissionDataSeeder;

    public ManagerRoleProvisioner(
        IdentityRoleManager roleManager,
        IPermissionDataSeeder permissionDataSeeder)
    {
        _roleManager = roleManager;
        _permissionDataSeeder = permissionDataSeeder;
    }

    /// <summary>Returns the manager role of the current tenant, creating (and granting) it if missing.</summary>
    public virtual async Task<IdentityRole> EnsureAsync()
    {
        var existing = await _roleManager.FindByNameAsync(TrackerRoles.Manager);
        if (existing != null)
        {
            return existing;
        }

        var role = new IdentityRole(GuidGenerator.Create(), TrackerRoles.Manager, CurrentTenant.Id)
        {
            // Static: cannot be renamed/deleted (membership changes assign it by name); still editable permissions.
            IsStatic = true,
            IsPublic = true
        };
        (await _roleManager.CreateAsync(role)).CheckErrors();

        await _permissionDataSeeder.SeedAsync(
            RolePermissionValueProvider.ProviderName,
            TrackerRoles.Manager,
            TrackerRoles.ManagerPermissions,
            CurrentTenant.Id);

        Logger.LogInformation("Created the '{Role}' role for tenant {TenantId}.", TrackerRoles.Manager, CurrentTenant.Id);
        return role;
    }
}
