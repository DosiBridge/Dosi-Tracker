using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Erasure;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Identity;
using Volo.Abp;
using Volo.Abp.Identity;
using Volo.Abp.Uow;
using Volo.Abp.Users;

namespace Dosi.Tracker.Identity;

/// <inheritdoc cref="IMyAccountAppService"/>
[Authorize]
[WorkspaceNotRequired]
[RemoteService(IsEnabled = false)]
public class MyAccountAppService : TrackerAppService, IMyAccountAppService
{
    /// <summary>The role of platform administrators (seeded by ABP), whose accounts are not deleted by self-service.</summary>
    public const string AdministratorRoleName = "admin";

    private readonly IdentityUserManager _userManager;
    private readonly AccountLinkEmails _linkEmails;
    private readonly AccountEraser _accountEraser;

    public MyAccountAppService(IdentityUserManager userManager, AccountLinkEmails linkEmails, AccountEraser accountEraser)
    {
        _userManager = userManager;
        _linkEmails = linkEmails;
        _accountEraser = accountEraser;
    }

    public async Task<EmailConfirmationSentDto> SendEmailConfirmationAsync()
    {
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        if (user.EmailConfirmed)
        {
            return new EmailConfirmationSentDto { AlreadyConfirmed = true };
        }

        return new EmailConfirmationSentDto { EmailSent = await _linkEmails.SendEmailConfirmationAsync(user) };
    }

    [AllowAnonymous]
    public async Task ConfirmEmailAsync(ConfirmEmailDto input)
    {
        var user = await _userManager.FindByIdAsync(input.UserId.ToString());
        if (user == null || !user.IsActive)
        {
            throw new BusinessException(TrackerDomainErrorCodes.EmailConfirmationInvalid);
        }

        if (user.EmailConfirmed)
        {
            return; // The link was opened twice.
        }

        var result = await _userManager.ConfirmEmailAsync(user, input.Token);
        if (!result.Succeeded)
        {
            throw new BusinessException(TrackerDomainErrorCodes.EmailConfirmationInvalid);
        }
    }

    public async Task<AccountDeletionCheckDto> GetDeletionCheckAsync()
    {
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        var isAdministrator = await _userManager.IsInRoleAsync(user, AdministratorRoleName);
        var soleOwned = await _accountEraser.FindSoleOwnedWorkspacesAsync(user.Id);

        return new AccountDeletionCheckDto
        {
            CanDelete = !isAdministrator && soleOwned.Count == 0,
            IsPlatformAdministrator = isAdministrator,
            SoleOwnerOf = soleOwned.Select(w => new OwnedWorkspaceDto { Id = w.Id, Name = w.Name }).ToList()
        };
    }

    public async Task DeleteAsync(DeleteMyAccountDto input)
    {
        var user = await _userManager.GetByIdAsync(CurrentUser.GetId());
        if (!await _userManager.CheckPasswordAsync(user, input.Password))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvalidPassword);
        }

        if (await _userManager.IsInRoleAsync(user, AdministratorRoleName))
        {
            throw new BusinessException(TrackerDomainErrorCodes.AccountDeletionNotAllowed);
        }

        using var uow = UnitOfWorkManager.Begin(new AbpUnitOfWorkOptions(isTransactional: true), requiresNew: true);
        await _accountEraser.DeleteAsync(user.Id);
        await uow.CompleteAsync();
    }
}
