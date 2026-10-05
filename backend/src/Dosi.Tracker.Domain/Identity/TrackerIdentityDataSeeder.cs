using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Replaces ABP's <see cref="IdentityDataSeeder"/>, which creates an <c>admin</c> user (with the given or the template
/// password) whenever no user is literally named "admin". Every DbMigrator run seeds the host and every workspace, so a
/// workspace whose owner renamed or deleted the default <c>admin</c> account would silently get a new full-admin
/// <c>admin</c> / <c>1q2w3E*</c> account back. This seeder creates the administrator only for a brand-new host or
/// workspace: one that has no users at all (soft-deleted users included, so deleting everyone does not re-open the door).
/// Within <see cref="SuppressAdminCreation"/> it never creates one. An administrator created with the template password
/// (which only a Development DbMigrator run accepts for the host) must change it at the first sign-in.
/// </summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(IIdentityDataSeeder), typeof(IdentityDataSeeder), typeof(TrackerIdentityDataSeeder))]
public class TrackerIdentityDataSeeder : IdentityDataSeeder
{
    /// <summary>The user name ABP's seeder gives the administrator when none is specified.</summary>
    public const string DefaultAdminUserName = "admin";

    private static readonly AsyncLocal<bool> AdminCreationSuppressed = new();

    protected IDataFilter DataFilter { get; }

    public ILogger<TrackerIdentityDataSeeder> Logger { get; set; }

    public TrackerIdentityDataSeeder(
        IGuidGenerator guidGenerator,
        IIdentityRoleRepository roleRepository,
        IIdentityUserRepository userRepository,
        ILookupNormalizer lookupNormalizer,
        IdentityUserManager userManager,
        IdentityRoleManager roleManager,
        ICurrentTenant currentTenant,
        IOptions<IdentityOptions> identityOptions,
        IDataFilter dataFilter)
        : base(guidGenerator, roleRepository, userRepository, lookupNormalizer, userManager, roleManager, currentTenant, identityOptions)
    {
        DataFilter = dataFilter;
        Logger = NullLogger<TrackerIdentityDataSeeder>.Instance;
    }

    /// <summary>True inside a <see cref="SuppressAdminCreation"/> scope of the current async flow.</summary>
    public static bool IsAdminCreationSuppressed => AdminCreationSuppressed.Value;

    /// <summary>True for the well-known template password (<c>1q2w3E*</c>) every ABP solution starts with.</summary>
    public static bool IsTemplatePassword(string? password)
    {
        return password == IdentityDataSeedContributor.AdminPasswordDefaultValue;
    }

    /// <summary>
    /// Seeds without ever creating an administrator for as long as the returned scope is open (in the current async
    /// flow). The DbMigrator seeds existing workspaces inside it: their owners already exist, and a workspace without
    /// any user must not get an account with a template password either.
    /// </summary>
    public static IDisposable SuppressAdminCreation()
    {
        var previous = AdminCreationSuppressed.Value;
        AdminCreationSuppressed.Value = true;
        return new DisposeAction(() => AdminCreationSuppressed.Value = previous);
    }

    [UnitOfWork]
    public override async Task<IdentityDataSeedResult> SeedAsync(
        string adminEmail,
        string adminPassword,
        Guid? tenantId = null,
        string? adminUserName = null)
    {
        if (IsAdminCreationSuppressed)
        {
            Logger.LogDebug("Administrator seeding is suppressed for tenant {TenantId}.", tenantId);
            return new IdentityDataSeedResult();
        }

        if (await HasAnyUserAsync(tenantId))
        {
            Logger.LogDebug("Tenant {TenantId} already has users; no administrator is seeded.", tenantId);
            return new IdentityDataSeedResult();
        }

        var templatePassword = IsTemplatePassword(adminPassword);
        if (templatePassword)
        {
            Logger.LogWarning(
                "Creating the administrator of tenant {TenantId} with the well-known template password; it must be changed at " +
                "the first sign-in (for the host, set App:HostAdminPassword before the first DbMigrator run).", tenantId);
        }

        var result = await base.SeedAsync(adminEmail, adminPassword, tenantId, adminUserName);

        if (result.CreatedAdminUser && templatePassword)
        {
            await RequirePasswordChangeAsync(tenantId, string.IsNullOrWhiteSpace(adminUserName) ? DefaultAdminUserName : adminUserName);
        }

        return result;
    }

    /// <summary>Whether the tenant (or the host, for null) has ever had a user: deleted users count too. A tenant without
    /// one is brand-new, the only kind this seeder creates an administrator for.</summary>
    [UnitOfWork]
    public virtual async Task<bool> HasAnyUserAsync(Guid? tenantId)
    {
        using (CurrentTenant.Change(tenantId))
        using (DataFilter.Disable<ISoftDelete>())
        {
            var users = await UserRepository.GetListAsync(maxResultCount: 1);
            return users.Count > 0;
        }
    }

    /// <summary>Makes the just-created administrator change the template password at the first sign-in.</summary>
    protected virtual async Task RequirePasswordChangeAsync(Guid? tenantId, string userName)
    {
        using (CurrentTenant.Change(tenantId))
        {
            var admin = await UserManager.FindByNameAsync(userName);
            if (admin == null)
            {
                return;
            }

            admin.SetShouldChangePasswordOnNextLogin(true);
            (await UserManager.UpdateAsync(admin)).CheckErrors();
        }
    }
}
