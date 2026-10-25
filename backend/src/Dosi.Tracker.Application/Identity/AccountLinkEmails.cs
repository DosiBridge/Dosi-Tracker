using System;
using System.Threading.Tasks;
using Dosi.Tracker.Emailing;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.Account;
using Volo.Abp.Account.Emailing;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Identity;
using Volo.Abp.UI.Navigation.Urls;

namespace Dosi.Tracker.Identity;

/// <summary>
/// The account emails that carry a one-time link: a password reset and an email confirmation. The links open the web
/// app (<see cref="WebAppLinks"/>); a password reset falls back to the API host's own reset page when no web app address
/// is configured. Best effort, like every email (<see cref="TrackerEmailer"/>).
/// </summary>
public class AccountLinkEmails : ITransientDependency
{
    private readonly IdentityUserManager _userManager;
    private readonly TrackerEmailer _emailer;
    private readonly IAppUrlProvider _appUrlProvider;

    public AccountLinkEmails(IdentityUserManager userManager, TrackerEmailer emailer, IAppUrlProvider appUrlProvider)
    {
        _userManager = userManager;
        _emailer = emailer;
        _appUrlProvider = appUrlProvider;
    }

    /// <summary>Emails <paramref name="user"/> a link to choose a new password; true when it was queued.</summary>
    public virtual async Task<bool> SendPasswordResetAsync(IdentityUser user, string? returnUrl = null)
    {
        if (string.IsNullOrWhiteSpace(user.Email))
        {
            return false;
        }

        var current = await CurrentAsync(user);
        var token = await _userManager.GeneratePasswordResetTokenAsync(current);
        return await SendPasswordResetAsync(current, token, returnUrl);
    }

    /// <summary>Emails <paramref name="user"/> the reset link for an already generated <paramref name="resetToken"/>.</summary>
    public virtual async Task<bool> SendPasswordResetAsync(IdentityUser user, string resetToken, string? returnUrl)
    {
        var link = _emailer.Links.ResetPassword(user.Id, resetToken, returnUrl)
                   ?? await FindHostResetLinkAsync(user.Id, resetToken);
        return link != null && await _emailer.QueuePasswordResetAsync(user.Email, link);
    }

    /// <summary>Emails <paramref name="user"/> a link that confirms their address; true when it was queued. Nothing is
    /// sent for an address that is confirmed already.</summary>
    public virtual async Task<bool> SendEmailConfirmationAsync(IdentityUser user)
    {
        if (user.EmailConfirmed || string.IsNullOrWhiteSpace(user.Email))
        {
            return false;
        }

        var current = await CurrentAsync(user);
        var token = await _userManager.GenerateEmailConfirmationTokenAsync(current);
        return await _emailer.QueueEmailConfirmationAsync(current.Email, current.Id, token);
    }

    /// <summary>The account as the current unit of work tracks it. ABP's token providers record the token they issue
    /// on the account and save it, which fails for an instance another unit of work loaded (a just-created account).</summary>
    private Task<IdentityUser> CurrentAsync(IdentityUser user) => _userManager.GetByIdAsync(user.Id);

    /// <summary>The API host's own reset page (ABP's account pages), or null when its address is not configured
    /// (<c>App:SelfUrl</c>).</summary>
    private async Task<string?> FindHostResetLinkAsync(Guid userId, string resetToken)
    {
        string page;
        try
        {
            page = await _appUrlProvider.GetResetPasswordUrlAsync("MVC");
        }
        catch (Exception exception)
        {
            Logger.LogWarning(exception, "No password reset email was sent: neither {ClientUrlKey} nor App:SelfUrl is configured.",
                WebAppLinks.ClientUrlKey);
            return null;
        }

        return string.IsNullOrWhiteSpace(page)
            ? null
            : $"{page}?userId={userId:D}&resetToken={Uri.EscapeDataString(resetToken)}";
    }

    public ILogger<AccountLinkEmails> Logger { get; set; } = NullLogger<AccountLinkEmails>.Instance;
}

/// <summary>ABP's account emailer, replaced: the reset link of <c>POST /api/account/send-password-reset-code</c> opens
/// the web app's reset page (or the host's own page when no web app is configured), the return URL may only be a path
/// on that app, and the email is queued for background delivery with retries.</summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(IAccountEmailer), typeof(TrackerAccountEmailer))]
public class TrackerAccountEmailer : IAccountEmailer, ITransientDependency
{
    private readonly AccountLinkEmails _linkEmails;

    public TrackerAccountEmailer(AccountLinkEmails linkEmails)
    {
        _linkEmails = linkEmails;
    }

    public virtual Task SendPasswordResetLinkAsync(
        IdentityUser user, string resetToken, string appName, string? returnUrl = null, string? returnUrlHash = null)
    {
        return _linkEmails.SendPasswordResetAsync(user, resetToken, returnUrl);
    }
}
