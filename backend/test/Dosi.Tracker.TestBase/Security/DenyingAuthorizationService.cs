using System;
using System.Collections.Generic;
using System.Security.Claims;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Volo.Abp.Authorization;
using Volo.Abp.Security.Claims;

namespace Dosi.Tracker.Security;

/// <summary>The permission names a <see cref="DenyingAuthorizationService"/> refuses.</summary>
public sealed class DeniedPermissions
{
    public DeniedPermissions(IEnumerable<string> names)
    {
        Names = new HashSet<string>(names, StringComparer.Ordinal);
    }

    public ISet<string> Names { get; }
}

/// <summary>
/// Test double mirroring ABP's AlwaysAllowAuthorizationService, except that the configured permissions are
/// denied. Lets integration tests exercise the "regular member" code paths of app services that branch on
/// <c>AuthorizationService.IsGrantedAsync(...)</c>, without a real role/permission store.
/// </summary>
public class DenyingAuthorizationService : IAbpAuthorizationService
{
    private readonly ICurrentPrincipalAccessor _currentPrincipalAccessor;
    private readonly DeniedPermissions _denied;

    public DenyingAuthorizationService(
        IServiceProvider serviceProvider,
        ICurrentPrincipalAccessor currentPrincipalAccessor,
        DeniedPermissions denied)
    {
        ServiceProvider = serviceProvider;
        _currentPrincipalAccessor = currentPrincipalAccessor;
        _denied = denied;
    }

    public IServiceProvider ServiceProvider { get; }

    public ClaimsPrincipal CurrentPrincipal => _currentPrincipalAccessor.Principal;

    public Task<AuthorizationResult> AuthorizeAsync(
        ClaimsPrincipal user, object? resource, IEnumerable<IAuthorizationRequirement> requirements)
    {
        return Task.FromResult(AuthorizationResult.Success());
    }

    public Task<AuthorizationResult> AuthorizeAsync(ClaimsPrincipal user, object? resource, string policyName)
    {
        return Task.FromResult(_denied.Names.Contains(policyName)
            ? AuthorizationResult.Failed()
            : AuthorizationResult.Success());
    }
}

public static class DenyingAuthorizationServiceCollectionExtensions
{
    /// <summary>Call from a test's <c>AfterAddApplication</c> override to act as a caller lacking these permissions.</summary>
    public static IServiceCollection DenyPermissions(this IServiceCollection services, params string[] permissionNames)
    {
        services.AddSingleton(new DeniedPermissions(permissionNames));
        services.Replace(ServiceDescriptor.Singleton<IAuthorizationService, DenyingAuthorizationService>());
        services.Replace(ServiceDescriptor.Singleton<IAbpAuthorizationService, DenyingAuthorizationService>());
        return services;
    }
}
