using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Authorization.Permissions;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.PermissionManagement;
using Volo.Abp.Uow;
using IdentityRole = Volo.Abp.Identity.IdentityRole;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Replaces ABP's <see cref="IdentityDataSeeder"/>: creates the platform administrator (global role
/// <see cref="TrackerRoles.PlatformAdmin"/>) only for a brand-new database — one without any user, deleted users
/// included — so a renamed or deleted administrator is never re-created with a template password. Within
/// <see cref="SuppressAdminCreation"/> it never creates one. An administrator created with the template password must
/// change it at the first sign-in. Workspace owners are ordinary users: they get their rights from their workspace
/// membership, never from an Identity role.
/// </summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(IIdentityDataSeeder), typeof(IdentityDataSeeder), typeof(TrackerIdentityDataSeeder))]
public class TrackerIdentityDataSeeder : IdentityDataSeeder
{
    public const string DefaultAdminUserName = "admin";

    private static readonly AsyncLocal<bool> AdminCreationSuppressed = new();

    protected IDataFilter DataFilter { get; }
    protected IPermissionDataSeeder PermissionDataSeeder { get; }
    protected IPermissionDefinitionManager PermissionDefinitionManager { get; }

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
        IDataFilter dataFilter,
        IPermissionDataSeeder permissionDataSeeder,
        IPermissionDefinitionManager permissionDefinitionManager)
        : base(guidGenerator, roleRepository, userRepository, lookupNormalizer, userManager, roleManager, currentTenant, identityOptions)
    {
        DataFilter = dataFilter;
        PermissionDataSeeder = permissionDataSeeder;
        PermissionDefinitionManager = permissionDefinitionManager;
        Logger = NullLogger<TrackerIdentityDataSeeder>.Instance;
    }

    public static bool IsAdminCreationSuppressed => AdminCreationSuppressed.Value;

    /// <summary>True for the well-known template password (<c>1q2w3E*</c>) every ABP solution starts with.</summary>
    public static bool IsTemplatePassword(string? password)
    {
        return password == IdentityDataSeedContributor.AdminPasswordDefaultValue;
    }

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
        var result = new IdentityDataSeedResult();
        var role = await EnsurePlatformAdminRoleAsync();
        result.CreatedAdminRole = role.Created;

        if (IsAdminCreationSuppressed)
        {
            Logger.LogDebug("Administrator seeding is suppressed.");
            return result;
        }

        if (await HasAnyUserAsync())
        {
            Logger.LogDebug("Users exist already; no administrator is seeded.");
            return result;
        }

        var templatePassword = IsTemplatePassword(adminPassword);
        if (templatePassword)
        {
            Logger.LogWarning(
                "Creating the platform administrator with the well-known template password; it must be changed at the first " +
                "sign-in (set App:HostAdminPassword before the first DbMigrator run).");
        }

        await IdentityOptions.SetAsync();
        var userName = string.IsNullOrWhiteSpace(adminUserName) ? DefaultAdminUserName : adminUserName;
        var admin = new IdentityUser(GuidGenerator.Create(), userName, adminEmail)
        {
            Name = userName
        };
        admin.SetEmailConfirmed(true);
        admin.SetShouldChangePasswordOnNextLogin(templatePassword);
        (await UserManager.CreateAsync(admin, adminPassword, validatePassword: false)).CheckErrors();
        (await UserManager.AddToRoleAsync(admin, TrackerRoles.PlatformAdmin)).CheckErrors();
        result.CreatedAdminUser = true;

        return result;
    }

    /// <summary>Creates the global <see cref="TrackerRoles.PlatformAdmin"/> role when missing and grants it the platform
    /// console and the host-side administration permissions (identity, settings, features, …). The workspace-side
    /// Tracker permissions come from workspace membership only.</summary>
    [UnitOfWork]
    public virtual async Task<(IdentityRole Role, bool Created)> EnsurePlatformAdminRoleAsync()
    {
        var role = await RoleManager.FindByNameAsync(TrackerRoles.PlatformAdmin);
        var created = false;
        if (role == null)
        {
            role = new IdentityRole(GuidGenerator.Create(), TrackerRoles.PlatformAdmin)
            {
                IsStatic = true,
                IsPublic = false
            };
            (await RoleManager.CreateAsync(role)).CheckErrors();
            created = true;
        }

        var permissions = (await PermissionDefinitionManager.GetPermissionsAsync())
            .Where(p => p.IsEnabled)
            .Select(p => p.Name)
            .Where(name => !name.StartsWith(TrackerPermissions.GroupName + ".", StringComparison.Ordinal) ||
                           name == TrackerPermissions.Platform.Default)
            .ToArray();
        await PermissionDataSeeder.SeedAsync(RolePermissionValueProvider.ProviderName, TrackerRoles.PlatformAdmin, permissions);

        return (role, created);
    }

    /// <summary>Whether any user ever existed (deleted users count too).</summary>
    [UnitOfWork]
    public virtual async Task<bool> HasAnyUserAsync()
    {
        using (DataFilter.Disable<ISoftDelete>())
        {
            var users = await UserRepository.GetListAsync(maxResultCount: 1);
            return users.Count > 0;
        }
    }
}
