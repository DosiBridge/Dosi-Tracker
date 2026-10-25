using System;
using System.Linq;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using OpenIddict.Abstractions;
using Shouldly;
using Volo.Abp;
using Volo.Abp.BlobStoring;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Security.Claims;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications.Identity;

/// <summary>
/// Self-service account deletion: confirmed with the password, refused while the account is a workspace's only owner (and
/// for platform administrators). The account leaves every workspace, every session is revoked, the identity is
/// anonymized — the address can register again — and the erasure sweep deletes its captures and scrubs its time blocks
/// of titles, application names, notes and device names, keeping the work-time records.
/// </summary>
// Runs a background sweep, whose named lock is process-wide (ABP's local lock): never alongside the other sweepers.
[Xunit.Collection(TrackerTestConsts.CollectionDefinitionName)]
public class AccountDeletionTests : TrackerEntityFrameworkCoreTestBase
{
    private const string Password = "1q2w3E*deletion";

    private readonly ScriptedBlobProvider _blobProvider = new();
    private readonly IMyAccountAppService _myAccount;
    private readonly IdentityUserManager _userManager;
    private readonly ICurrentPrincipalAccessor _principalAccessor;
    private readonly IGuidGenerator _guidGenerator;
    private readonly IClock _clock;

    public AccountDeletionTests()
    {
        _myAccount = GetRequiredService<IMyAccountAppService>();
        _userManager = GetRequiredService<IdentityUserManager>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _clock = GetRequiredService<IClock>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
        services.AddSingleton(_blobProvider);
        services.AddSingleton<IBlobProvider>(_blobProvider);
        services.Configure<AbpBlobStoringOptions>(options =>
            options.Containers.ConfigureDefault(container => container.ProviderType = typeof(ScriptedBlobProvider)));
    }

    private IDisposable ActAs(Guid userId) =>
        _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(new[] { new Claim(AbpClaimTypes.UserId, userId.ToString()) }, "Test")));

    private Task<IdentityUser> CreateUserAsync(string email)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(_guidGenerator.Create(), email, email) { Name = "Dana", Surname = "Doe" };
            (await _userManager.CreateAsync(user, Password)).Succeeded.ShouldBeTrue();
            (await _userManager.SetPhoneNumberAsync(user, "+15550100")).Succeeded.ShouldBeTrue();
            return user;
        });
    }

    private Task<IdentityUser?> FindIncludingDeletedAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<ISoftDelete>())
            {
                return await GetRequiredService<IRepository<IdentityUser, Guid>>().FindAsync(userId);
            }
        });
    }

    private Task<int> CountAsync<TEntity>(Func<TEntity, bool> predicate) where TEntity : class, Volo.Abp.Domain.Entities.IEntity
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return (await GetRequiredService<IRepository<TEntity>>().GetListAsync()).Count(predicate);
            }
        });
    }

    /// <summary>A block of the user in the workspace, with window titles, a note, a device name, a timeline with
    /// application names and a screen capture (row and file).</summary>
    private async Task<Guid> SeedBlockAsync(Guid workspaceId, Guid userId)
    {
        var activityId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
            {
                var projectId = _guidGenerator.Create();
                await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(new Project(projectId, workspaceId, "Project"), autoSave: true);
                await GetRequiredService<IRepository<ProjectMember, Guid>>()
                    .InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectId, userId), autoSave: true);
                var start = _clock.Now.AddDays(-1);
                await GetRequiredService<IRepository<Activity, Guid>>().InsertAsync(new Activity(
                    activityId, workspaceId, userId, projectId, _guidGenerator.Create(), start, start.AddMinutes(10))
                {
                    KeyboardHits = 120,
                    MouseClicks = 40,
                    Description = "Salary review for J. Smith",
                    DeviceName = "DANA-LAPTOP",
                    ActiveWindowsJson = "[{\"AppName\":\"Excel\",\"WindowTitle\":\"salaries.xlsx\"}]",
                    RunningProgramsJson = "[{\"AppName\":\"Excel\"}]",
                    TimelineJson = "[{\"Minute\":0,\"KeyboardHits\":12,\"Active\":true,\"AppName\":\"Excel\"}]"
                }, autoSave: true);
                var blobName = $"activities/{activityId:D}/screen.jpg";
                _blobProvider.Blobs[blobName] = new byte[] { 0xFF, 0xD8 };
                await GetRequiredService<IRepository<Screenshot, Guid>>().InsertAsync(new Screenshot(
                    _guidGenerator.Create(), workspaceId, activityId, blobName, start.AddMinutes(5), 2), autoSave: true);
                await GetRequiredService<IRepository<AgentPresence, Guid>>().InsertAsync(
                    new AgentPresence(_guidGenerator.Create(), workspaceId, userId, Guid.NewGuid(), "active", _clock.Now), autoSave: true);
            }
        });

        return activityId;
    }

    private async Task<string> SignInAsync(Guid userId)
    {
        return await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            var authorizations = GetRequiredService<IOpenIddictAuthorizationManager>();
            var authorization = await authorizations.CreateAsync(new OpenIddictAuthorizationDescriptor
            {
                CreationDate = DateTimeOffset.UtcNow,
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = userId.ToString(),
                Type = OpenIddictConstants.AuthorizationTypes.AdHoc
            });
            var token = await tokens.CreateAsync(new OpenIddictTokenDescriptor
            {
                AuthorizationId = await authorizations.GetIdAsync(authorization),
                CreationDate = DateTimeOffset.UtcNow,
                ExpirationDate = DateTimeOffset.UtcNow.AddDays(14),
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = userId.ToString(),
                Type = OpenIddictConstants.TokenTypeHints.RefreshToken
            });
            return (await tokens.GetIdAsync(token))!;
        });
    }

    private DataErasureWorker NewWorker() => new(
        GetRequiredService<AbpAsyncTimer>(),
        GetRequiredService<IServiceScopeFactory>(),
        Options.Create(new DataErasureOptions { BatchSize = 1 }))
    {
        LazyServiceProvider = GetRequiredService<Volo.Abp.DependencyInjection.IAbpLazyServiceProvider>()
    };

    [Fact]
    public async Task The_Only_Owner_Of_A_Workspace_Cannot_Delete_Their_Account_Until_Someone_Else_Owns_It()
    {
        var owner = await CreateUserAsync("sole-owner@deletion.test");
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Sole Owned Co");
        await ServiceProvider.AddWorkspaceMemberAsync(owner.Id, WorkspaceRoles.Owner, workspaceId);

        using (ActAs(owner.Id))
        {
            var check = await _myAccount.GetDeletionCheckAsync();
            check.CanDelete.ShouldBeFalse();
            check.IsPlatformAdministrator.ShouldBeFalse();
            check.SoleOwnerOf.ShouldHaveSingleItem().Name.ShouldBe("Sole Owned Co");

            var refused = await Should.ThrowAsync<BusinessException>(() => _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password }));
            refused.Code.ShouldBe(TrackerDomainErrorCodes.SoleWorkspaceOwner);
            refused.Data["workspaces"].ShouldBe("Sole Owned Co");
        }

        var coOwner = await CreateUserAsync("co-owner@deletion.test");
        await ServiceProvider.AddWorkspaceMemberAsync(coOwner.Id, WorkspaceRoles.Owner, workspaceId);

        using (ActAs(owner.Id))
        {
            (await _myAccount.GetDeletionCheckAsync()).CanDelete.ShouldBeTrue();
            await _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password });
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(owner.Id, workspaceId))!.IsActive.ShouldBeFalse();
        (await ServiceProvider.FindWorkspaceMemberAsync(coOwner.Id, workspaceId))!.IsOwner.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Wrong_Password_Or_A_Platform_Administrator_Is_Refused()
    {
        var user = await CreateUserAsync("careful@deletion.test");
        var admin = await CreateUserAsync("platform-admin@deletion.test");
        await WithUnitOfWorkAsync(async () =>
        {
            var roles = GetRequiredService<IdentityRoleManager>();
            if (await roles.FindByNameAsync(MyAccountAppService.AdministratorRoleName) == null)
            {
                (await roles.CreateAsync(new IdentityRole(_guidGenerator.Create(), MyAccountAppService.AdministratorRoleName)))
                    .Succeeded.ShouldBeTrue();
            }

            var reloaded = await _userManager.GetByIdAsync(admin.Id);
            (await _userManager.AddToRoleAsync(reloaded, MyAccountAppService.AdministratorRoleName)).Succeeded.ShouldBeTrue();
        });

        using (ActAs(user.Id))
        {
            (await Should.ThrowAsync<BusinessException>(() => _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = "wrong" })))
                .Code.ShouldBe(TrackerDomainErrorCodes.InvalidPassword);
        }

        using (ActAs(admin.Id))
        {
            (await _myAccount.GetDeletionCheckAsync()).IsPlatformAdministrator.ShouldBeTrue();
            (await Should.ThrowAsync<BusinessException>(() => _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password })))
                .Code.ShouldBe(TrackerDomainErrorCodes.AccountDeletionNotAllowed);
        }

        (await FindIncludingDeletedAsync(user.Id))!.IsDeleted.ShouldBeFalse();
        (await FindIncludingDeletedAsync(admin.Id))!.IsDeleted.ShouldBeFalse();
    }

    [Fact]
    public async Task Deleting_An_Account_Ends_Its_Memberships_And_Sessions_And_Anonymizes_It_At_Once()
    {
        var user = await CreateUserAsync("leaving@deletion.test");
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Leaving Co");
        await ServiceProvider.AddWorkspaceMemberAsync(user.Id, WorkspaceRoles.Member, workspaceId);
        await SeedBlockAsync(workspaceId, user.Id);
        var refreshTokenId = await SignInAsync(user.Id);

        using (ActAs(user.Id))
        {
            await _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password });
        }

        (await ServiceProvider.FindWorkspaceMemberAsync(user.Id, workspaceId))!.IsActive.ShouldBeFalse();
        (await CountAsync<ProjectMember>(m => m.UserId == user.Id)).ShouldBe(0);
        (await CountAsync<AgentPresence>(p => p.UserId == user.Id)).ShouldBe(0);
        await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(refreshTokenId))!)).ShouldBe(OpenIddictConstants.Statuses.Revoked);
        });

        var deleted = (await FindIncludingDeletedAsync(user.Id)).ShouldNotBeNull();
        deleted.IsDeleted.ShouldBeTrue();
        deleted.IsActive.ShouldBeFalse();
        deleted.UserName.ShouldBe(AccountEraser.PlaceholderUserName(user.Id));
        deleted.Email.ShouldBe($"{AccountEraser.PlaceholderUserName(user.Id)}@{AccountEraser.PlaceholderEmailDomain}");
        deleted.Name.ShouldBeNull();
        deleted.Surname.ShouldBeNull();
        deleted.PhoneNumber.ShouldBeNull();
        (await WithUnitOfWorkAsync(() => _userManager.FindByEmailAsync("leaving@deletion.test"))).ShouldBeNull();
        (await CountAsync<AccountErasure>(e => e.UserId == user.Id && e.CompletedTime == null)).ShouldBe(1);

        // The address is free: the person can sign up again.
        var again = await CreateUserAsync("leaving@deletion.test");
        again.Id.ShouldNotBe(user.Id);
    }

    [Fact]
    public async Task The_Sweep_Deletes_A_Deleted_Accounts_Captures_And_Scrubs_Its_Blocks_But_Keeps_The_Time()
    {
        var user = await CreateUserAsync("scrub-me@deletion.test");
        var colleague = await CreateUserAsync("colleague@deletion.test");
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Scrub Co");
        await ServiceProvider.AddWorkspaceMemberAsync(user.Id, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(colleague.Id, WorkspaceRoles.Owner, workspaceId);
        var userBlock = await SeedBlockAsync(workspaceId, user.Id);
        var colleagueBlock = await SeedBlockAsync(workspaceId, colleague.Id);

        using (ActAs(user.Id))
        {
            await _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password });
        }

        var result = await NewWorker().SweepAsync(ServiceProvider);

        result.AccountsScrubbed.ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.ActivityId == userBlock)).ShouldBe(0);
        _blobProvider.Blobs.ContainsKey($"activities/{userBlock:D}/screen.jpg").ShouldBeFalse();
        var scrubbed = await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await GetRequiredService<IRepository<Activity, Guid>>().GetAsync(userBlock);
            }
        });
        scrubbed.ActiveWindowsJson.ShouldBe("[]");
        scrubbed.RunningProgramsJson.ShouldBe("[]");
        scrubbed.Description.ShouldBeNull();
        scrubbed.DeviceName.ShouldBeNull();
        scrubbed.TimelineJson.ShouldBe("[{\"Minute\":0,\"KeyboardHits\":12,\"Active\":true}]");
        scrubbed.KeyboardHits.ShouldBe(120); // the work-time record stays
        (await CountAsync<AccountErasure>(e => e.UserId == user.Id && e.CompletedTime != null)).ShouldBe(1);

        // The colleague's data is untouched, and the next sweep has nothing to do.
        (await CountAsync<Screenshot>(s => s.ActivityId == colleagueBlock)).ShouldBe(1);
        (await CountAsync<Activity>(a => a.Id == colleagueBlock && a.Description != null)).ShouldBe(1);
        (await NewWorker().SweepAsync(ServiceProvider)).AccountsScrubbed.ShouldBe(0);
    }

    [Fact]
    public async Task A_Capture_File_That_Cannot_Be_Deleted_Leaves_The_Scrub_Unfinished_Until_A_Later_Sweep()
    {
        var user = await CreateUserAsync("store-down@deletion.test");
        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Store Down Scrub Co");
        await ServiceProvider.AddWorkspaceMemberAsync(user.Id, WorkspaceRoles.Member, workspaceId);
        var block = await SeedBlockAsync(workspaceId, user.Id);
        using (ActAs(user.Id))
        {
            await _myAccount.DeleteAsync(new DeleteMyAccountDto { Password = Password });
        }

        _blobProvider.BeforeDelete = (_, _) => throw new InvalidOperationException("the object store is down");
        var first = await NewWorker().SweepAsync(ServiceProvider);
        first.AccountsScrubbed.ShouldBe(0);
        first.Unfinished.ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.ActivityId == block)).ShouldBe(1);
        (await CountAsync<AccountErasure>(e => e.UserId == user.Id && e.CompletedTime == null)).ShouldBe(1);

        _blobProvider.BeforeDelete = null;
        (await NewWorker().SweepAsync(ServiceProvider)).AccountsScrubbed.ShouldBe(1);
        (await CountAsync<Screenshot>(s => s.ActivityId == block)).ShouldBe(0);
    }

    [Fact]
    public async Task A_Sweep_Is_Skipped_While_Another_One_Holds_The_Lock()
    {
        await using var held = await GetRequiredService<Volo.Abp.DistributedLocking.IAbpDistributedLock>()
            .TryAcquireAsync(DataErasureWorker.LockName);
        held.ShouldNotBeNull();

        (await NewWorker().SweepAsync(ServiceProvider)).Skipped.ShouldBeTrue();
    }

    [Fact]
    public async Task Revoking_Every_Session_Includes_Tokens_Without_An_Authorization()
    {
        var userId = Guid.NewGuid();
        var withAuthorization = await SignInAsync(userId);
        var orphanId = await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            return (await tokens.GetIdAsync(await tokens.CreateAsync(new OpenIddictTokenDescriptor
            {
                CreationDate = DateTimeOffset.UtcNow,
                ExpirationDate = DateTimeOffset.UtcNow.AddDays(1),
                Status = OpenIddictConstants.Statuses.Valid,
                Subject = userId.ToString(),
                Type = OpenIddictConstants.TokenTypeHints.RefreshToken
            })))!;
        });
        var someoneElse = await SignInAsync(Guid.NewGuid());

        (await WithUnitOfWorkAsync(() => GetRequiredService<Dosi.Tracker.Devices.DeviceSessionManager>().RevokeAllAsync(userId))).ShouldBe(1);

        await WithUnitOfWorkAsync(async () =>
        {
            var tokens = GetRequiredService<IOpenIddictTokenManager>();
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(withAuthorization))!)).ShouldBe(OpenIddictConstants.Statuses.Revoked);
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(orphanId))!)).ShouldBe(OpenIddictConstants.Statuses.Revoked);
            (await tokens.GetStatusAsync((await tokens.FindByIdAsync(someoneElse))!)).ShouldBe(OpenIddictConstants.Statuses.Valid);
        });
    }
}
