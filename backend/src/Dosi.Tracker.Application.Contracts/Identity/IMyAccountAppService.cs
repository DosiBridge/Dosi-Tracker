using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Threading.Tasks;
using Volo.Abp.Application.Services;
using Volo.Abp.Auditing;

namespace Dosi.Tracker.Identity;

/// <summary>
/// The signed-in person's own account (no <c>X-Workspace-Id</c> needed): confirming the email address, and deleting
/// the account. Password recovery is ABP's account API: <c>POST /api/account/send-password-reset-code</c> (answers the
/// same whether or not the address has an account) and <c>POST /api/account/reset-password</c>.
/// </summary>
public interface IMyAccountAppService : IApplicationService
{
    /// <summary><c>POST /api/app/my-account/send-email-confirmation</c>: emails the caller a link that confirms their
    /// address (nothing when it is confirmed already).</summary>
    Task<EmailConfirmationSentDto> SendEmailConfirmationAsync();

    /// <summary><c>POST /api/app/my-account/confirm-email</c> (anonymous): completes the link's confirmation;
    /// <c>Tracker:EmailConfirmationInvalid</c> for a wrong or expired link.</summary>
    Task ConfirmEmailAsync(ConfirmEmailDto input);

    /// <summary><c>GET /api/app/my-account/deletion-check</c>: whether the caller can delete their account now, and
    /// the workspaces that stop them (those they are the only owner of).</summary>
    Task<AccountDeletionCheckDto> GetDeletionCheckAsync();

    /// <summary><c>POST /api/app/my-account/delete</c>: deletes the caller's account, confirmed with their password
    /// (<c>Tracker:InvalidPassword</c>). Refused while they are the only owner of a workspace
    /// (<c>Tracker:SoleWorkspaceOwner</c>) and for platform administrators (<c>Tracker:AccountDeletionNotAllowed</c>).
    /// Every session ends at once; the person's captures are deleted and their time blocks scrubbed shortly after.</summary>
    Task DeleteAsync(DeleteMyAccountDto input);
}

public class EmailConfirmationSentDto
{
    /// <summary>The address was confirmed already: nothing was sent.</summary>
    public bool AlreadyConfirmed { get; set; }

    /// <summary>Whether the confirmation email was sent (false when this server sends no email).</summary>
    public bool EmailSent { get; set; }
}

public class ConfirmEmailDto
{
    [Required]
    public Guid UserId { get; set; }

    [Required]
    [MaxLength(2048)]
    [DisableAuditing]
    public string Token { get; set; } = string.Empty;
}

public class AccountDeletionCheckDto
{
    public bool CanDelete { get; set; }

    /// <summary>A platform administrator's account cannot be deleted here.</summary>
    public bool IsPlatformAdministrator { get; set; }

    /// <summary>The workspaces the caller is the only owner of: make someone else an owner, or delete them, first.</summary>
    public List<OwnedWorkspaceDto> SoleOwnerOf { get; set; } = new();
}

public class OwnedWorkspaceDto
{
    public Guid Id { get; set; }
    public string Name { get; set; } = string.Empty;
}

public class DeleteMyAccountDto
{
    [Required]
    [MaxLength(128)]
    [DisableAuditing]
    public string Password { get; set; } = string.Empty;
}
