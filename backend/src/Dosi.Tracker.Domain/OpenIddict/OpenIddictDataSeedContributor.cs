using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using OpenIddict.Abstractions;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.OpenIddict;
using Volo.Abp.OpenIddict.Applications;
using Volo.Abp.OpenIddict.Scopes;
using Volo.Abp.Uow;

namespace Dosi.Tracker.OpenIddict;

/// <summary>
/// Creates the OpenIddict data the clients need to talk to the server: the API scope and the client applications
/// (create or update, so every DbMigrator run also brings existing databases up to date).
/// <para><b>Tracker_App</b> — the public client of the web dashboard and both desktop agents (Windows, macOS). They sign
/// in with the <c>password</c> grant and renew the session with <c>refresh_token</c> instead of sending the password
/// again. Scopes they may request:</para>
/// <list type="bullet">
/// <item><c>Tracker</c> — the API itself (audience "Tracker"); every client requests it.</item>
/// <item><c>offline_access</c> — asks for a refresh token. Clients that keep a session alive (the agents for days, the
/// web beyond the one-hour access token) request it and later post <c>grant_type=refresh_token</c>; the sign-in rate
/// limit treats refreshes far more generously than password sign-ins.</item>
/// <item><c>profile</c>, <c>email</c>, <c>roles</c>, <c>phone</c>, <c>address</c> — identity claims in the tokens.</item>
/// </list>
/// <para><b>Tracker_Swagger</b> — the Swagger UI (authorization code flow, no refresh tokens), seeded when its client id
/// and root URL are configured.</para>
/// </summary>
public class OpenIddictDataSeedContributor : OpenIddictDataSeedContributorBase, IDataSeedContributor, ITransientDependency
{
    /// <summary>The API scope (and resource) every client requests.</summary>
    public const string ApiScope = "Tracker";

    public OpenIddictDataSeedContributor(
        IConfiguration configuration,
        IOpenIddictApplicationRepository openIddictApplicationRepository,
        IAbpApplicationManager applicationManager,
        IOpenIddictScopeRepository openIddictScopeRepository,
        IOpenIddictScopeManager scopeManager)
        : base(configuration, openIddictApplicationRepository, applicationManager, openIddictScopeRepository, scopeManager)
    {
    }

    [UnitOfWork]
    public virtual async Task SeedAsync(DataSeedContext context)
    {
        await CreateScopesAsync();
        await CreateApplicationsAsync();
    }

    /// <summary>Identity claims plus the API: what every client may request.</summary>
    public static List<string> CommonScopes() => new()
    {
        OpenIddictConstants.Permissions.Scopes.Address,
        OpenIddictConstants.Permissions.Scopes.Email,
        OpenIddictConstants.Permissions.Scopes.Phone,
        OpenIddictConstants.Permissions.Scopes.Profile,
        OpenIddictConstants.Permissions.Scopes.Roles,
        ApiScope
    };

    /// <summary>The scopes the web dashboard and the desktop agents may request: <see cref="CommonScopes"/> plus
    /// <c>offline_access</c> for refresh tokens (see the class summary).</summary>
    public static List<string> AppClientScopes()
    {
        var scopes = CommonScopes();
        scopes.Add(OpenIddictConstants.Scopes.OfflineAccess);
        return scopes;
    }

    /// <summary>The grants of the web dashboard and the desktop agents: password sign-in plus refresh tokens.</summary>
    public static List<string> AppClientGrantTypes() => new()
    {
        OpenIddictConstants.GrantTypes.AuthorizationCode,
        OpenIddictConstants.GrantTypes.Password,
        OpenIddictConstants.GrantTypes.ClientCredentials,
        OpenIddictConstants.GrantTypes.RefreshToken,
        "LinkLogin",
        "Impersonation"
    };

    private async Task CreateScopesAsync()
    {
        await CreateScopesAsync(new OpenIddictScopeDescriptor
        {
            Name = ApiScope,
            DisplayName = "Tracker API",
            Resources = { ApiScope }
        });
    }

    private async Task CreateApplicationsAsync()
    {
        var configurationSection = Configuration.GetSection("OpenIddict:Applications");

        // Web dashboard and desktop agents (also the console test client).
        var appClientId = configurationSection["Tracker_App:ClientId"];
        if (!appClientId.IsNullOrWhiteSpace())
        {
            var appClientRootUrl = configurationSection["Tracker_App:RootUrl"]?.TrimEnd('/');
            await CreateOrUpdateApplicationAsync(
                applicationType: OpenIddictConstants.ApplicationTypes.Web,
                name: appClientId!,
                type: OpenIddictConstants.ClientTypes.Public,
                consentType: OpenIddictConstants.ConsentTypes.Implicit,
                displayName: "Dosi Tracker web dashboard and desktop agents",
                secret: null,
                grantTypes: AppClientGrantTypes(),
                scopes: AppClientScopes(),
                redirectUris: UrlList(appClientRootUrl),
                postLogoutRedirectUris: UrlList(appClientRootUrl),
                clientUri: appClientRootUrl,
                logoUri: "/images/clients/angular.svg"
            );
        }

        // Swagger UI
        var swaggerClientId = configurationSection["Tracker_Swagger:ClientId"];
        var swaggerRootUrl = configurationSection["Tracker_Swagger:RootUrl"]?.TrimEnd('/');
        if (!swaggerClientId.IsNullOrWhiteSpace() && !swaggerRootUrl.IsNullOrWhiteSpace())
        {
            await CreateOrUpdateApplicationAsync(
                applicationType: OpenIddictConstants.ApplicationTypes.Web,
                name: swaggerClientId!,
                type: OpenIddictConstants.ClientTypes.Public,
                consentType: OpenIddictConstants.ConsentTypes.Implicit,
                displayName: "Swagger Application",
                secret: null,
                grantTypes: new List<string> { OpenIddictConstants.GrantTypes.AuthorizationCode },
                scopes: CommonScopes(),
                redirectUris: new List<string> { $"{swaggerRootUrl}/swagger/oauth2-redirect.html" },
                clientUri: swaggerRootUrl!.EnsureEndsWith('/') + "swagger",
                logoUri: "/images/clients/swagger.svg"
            );
        }
    }

    /// <summary>The URL as a one-item list, or an empty list when it is not configured (the agents need no redirect URI).</summary>
    private static List<string> UrlList(string? url)
    {
        return url.IsNullOrWhiteSpace() ? new List<string>() : new List<string> { url! };
    }
}
