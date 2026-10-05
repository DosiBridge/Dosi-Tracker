using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.OpenIddict;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using OpenIddict.Abstractions;
using Shouldly;
using Volo.Abp.Modularity;
using Volo.Abp.OpenIddict.Applications;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// The seeded OpenIddict clients: the web dashboard and the desktop agents share <c>Tracker_App</c>, which must allow the
/// password grant and refresh tokens (<c>offline_access</c>) so sessions can be renewed without re-sending the password.
/// </summary>
public abstract class OpenIddictClientSeedingTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void BeforeAddApplication(IServiceCollection services)
    {
        // The DbMigrator's settings: the app client without a root URL (the agents need none), Swagger with one.
        services.ReplaceConfiguration(new ConfigurationBuilder()
            .AddJsonFile("appsettings.json", false)
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["OpenIddict:Applications:Tracker_App:ClientId"] = "Tracker_App",
                ["OpenIddict:Applications:Tracker_Swagger:ClientId"] = "Tracker_Swagger",
                ["OpenIddict:Applications:Tracker_Swagger:RootUrl"] = "https://localhost:6006/"
            })
            .Build());
    }

    private Task<List<string>> PermissionsOfAsync(string clientId)
    {
        // The application seeding ran when the test application started.
        return WithUnitOfWorkAsync(async () =>
        {
            var application = await GetRequiredService<IOpenIddictApplicationRepository>().FindByClientIdAsync(clientId);
            application.ShouldNotBeNull();
            return JsonSerializer.Deserialize<List<string>>(application.Permissions!)!;
        });
    }

    [Fact]
    public async Task The_App_Client_Should_Allow_Password_Sign_In_And_Refresh_Tokens()
    {
        var permissions = await PermissionsOfAsync("Tracker_App");

        permissions.ShouldContain(OpenIddictConstants.Permissions.GrantTypes.Password);
        permissions.ShouldContain(OpenIddictConstants.Permissions.GrantTypes.RefreshToken);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Endpoints.Token);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Prefixes.Scope + OpenIddictConstants.Scopes.OfflineAccess);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Prefixes.Scope + OpenIddictDataSeedContributor.ApiScope);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Scopes.Profile);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Scopes.Email);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Scopes.Roles);
    }

    [Fact]
    public async Task The_Swagger_Client_Should_Not_Get_Refresh_Tokens()
    {
        var permissions = await PermissionsOfAsync("Tracker_Swagger");

        permissions.ShouldContain(OpenIddictConstants.Permissions.GrantTypes.AuthorizationCode);
        permissions.ShouldContain(OpenIddictConstants.Permissions.Prefixes.Scope + OpenIddictDataSeedContributor.ApiScope);
        permissions.ShouldNotContain(OpenIddictConstants.Permissions.GrantTypes.RefreshToken);
        permissions.ShouldNotContain(OpenIddictConstants.Permissions.Prefixes.Scope + OpenIddictConstants.Scopes.OfflineAccess);
    }

    [Fact]
    public void The_Documented_Scopes_And_Grants_Should_Match_What_The_Clients_Request()
    {
        // clients/windows (api.rs) and clients/macos (ApiClient.swift) request "Tracker" (+ "offline_access").
        OpenIddictDataSeedContributor.AppClientScopes().ShouldContain("Tracker");
        OpenIddictDataSeedContributor.AppClientScopes().ShouldContain(OpenIddictConstants.Scopes.OfflineAccess);
        OpenIddictDataSeedContributor.CommonScopes().ShouldNotContain(OpenIddictConstants.Scopes.OfflineAccess);
        OpenIddictDataSeedContributor.AppClientGrantTypes().ShouldContain(OpenIddictConstants.GrantTypes.Password);
        OpenIddictDataSeedContributor.AppClientGrantTypes().ShouldContain(OpenIddictConstants.GrantTypes.RefreshToken);
    }
}
