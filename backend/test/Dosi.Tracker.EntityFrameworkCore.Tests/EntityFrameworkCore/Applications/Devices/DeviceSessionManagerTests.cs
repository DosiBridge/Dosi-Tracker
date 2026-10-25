using System;
using System.Text.Json;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using OpenIddict.Abstractions;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Devices;

/// <summary>Revoking a device's sign-in sessions through OpenIddict's managers (<see cref="DeviceSessionManager"/>).</summary>
public class DeviceSessionManagerTests : TrackerEntityFrameworkCoreTestBase
{
    private static readonly Guid UserId = Guid.Parse("be701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Laptop = Guid.Parse("be701e62-0953-4dd3-910b-dc6cc93ccb02");

    [Fact]
    public async Task A_Revoked_Refresh_Token_No_Longer_Looks_Valid_Even_Through_OpenIddicts_Cache()
    {
        var (authorizationId, refreshTokenId, expiredTokenId) = await WithUnitOfWorkAsync(async () =>
        {
            var authorizations = GetRequiredService<IOpenIddictAuthorizationManager>();
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            var descriptor = new OpenIddictAuthorizationDescriptor
            {
                CreationDate = DateTimeOffset.UtcNow,
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = UserId.ToString(),
                Type = OpenIddictConstants.AuthorizationTypes.AdHoc
            };
            descriptor.Properties[DeviceConsts.AuthorizationProperty] = JsonSerializer.SerializeToElement(Laptop.ToString());
            var authorizationId = (await authorizations.GetIdAsync(await authorizations.CreateAsync(descriptor)))!;

            async Task<string> TokenAsync(DateTimeOffset expires) => (await tokens.GetIdAsync(await tokens.CreateAsync(new OpenIddictTokenDescriptor
            {
                AuthorizationId = authorizationId,
                CreationDate = DateTimeOffset.UtcNow.AddDays(-20),
                ExpirationDate = expires,
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = UserId.ToString(),
                Type = OpenIddictConstants.TokenTypeHints.RefreshToken
            })))!;

            return (authorizationId, await TokenAsync(DateTimeOffset.UtcNow.AddDays(14)), await TokenAsync(DateTimeOffset.UtcNow.AddDays(-1)));
        });

        // OpenIddict looks the token up (and caches it) when the install refreshes.
        await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            (await tokens.HasStatusAsync((await tokens.FindByIdAsync(refreshTokenId))!, OpenIddictConstants.Statuses.Valid)).ShouldBeTrue();
        });

        (await WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().RevokeAsync(UserId, Laptop))).ShouldBe(1);
        (await WithUnitOfWorkAsync(() => GetRequiredService<DeviceSessionManager>().RevokeAsync(UserId, Guid.NewGuid()))).ShouldBe(0);

        await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            var authorizations = GetRequiredService<IOpenIddictAuthorizationManager>();
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(refreshTokenId))!)).ShouldBe(OpenIddictConstants.Statuses.Revoked);
            (await authorizations.GetStatusAsync((await authorizations.FindByIdAsync(authorizationId))!)).ShouldBe(OpenIddictConstants.Statuses.Revoked);
            // An expired token is unusable anyway and is left for OpenIddict's pruning.
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(expiredTokenId))!)).ShouldBe(OpenIddictConstants.Statuses.Valid);
        });
    }
}
