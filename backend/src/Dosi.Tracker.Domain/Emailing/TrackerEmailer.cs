using System;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Emailing;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// Queues the transactional emails (<see cref="TrackerEmailTemplates"/>) through <see cref="IEmailSender"/>, which
/// delivers them in the background with retries. Email is best effort: every method returns whether the email was
/// queued, and never throws — no invitation, sign-up or sweep fails because an email could not be sent. Nothing is
/// queued when the deployment cannot deliver email (<see cref="IEmailDeliveryChecker"/>) or no web app address is known
/// for the email's link (<see cref="WebAppLinks"/>).
/// </summary>
public class TrackerEmailer : ITransientDependency
{
    private readonly IEmailSender _emailSender;
    private readonly IEmailDeliveryChecker _deliveryChecker;

    public TrackerEmailer(IEmailSender emailSender, IEmailDeliveryChecker deliveryChecker, WebAppLinks links)
    {
        _emailSender = emailSender;
        _deliveryChecker = deliveryChecker;
        Links = links;
        Logger = NullLogger<TrackerEmailer>.Instance;
    }

    public ILogger<TrackerEmailer> Logger { get; set; }

    public WebAppLinks Links { get; }

    public virtual Task<bool> QueueInvitationAsync(
        string to, string workspaceName, string? inviterName, string role, string token, DateTime expiresAt)
    {
        var link = Links.Invitation(token);
        return link == null
            ? NoLinkAsync("invitation")
            : QueueAsync(to, TrackerEmailTemplates.Invitation(workspaceName, inviterName, role, link, expiresAt));
    }

    /// <param name="resetLink">The page that completes the reset (<see cref="WebAppLinks.ResetPassword"/>, or the
    /// API host's own account page when no web app is known).</param>
    public virtual Task<bool> QueuePasswordResetAsync(string to, string resetLink)
    {
        return QueueAsync(to, TrackerEmailTemplates.PasswordReset(resetLink));
    }

    public virtual Task<bool> QueueEmailConfirmationAsync(string to, Guid userId, string token)
    {
        var link = Links.ConfirmEmail(userId, token);
        return link == null
            ? NoLinkAsync("email confirmation")
            : QueueAsync(to, TrackerEmailTemplates.EmailConfirmation(link));
    }

    public virtual Task<bool> QueueTrialEndingAsync(string to, string workspaceName, string planName, DateTime endsAt)
    {
        return QueueAsync(to, TrackerEmailTemplates.TrialEnding(workspaceName, planName, endsAt, Links.Billing()));
    }

    public virtual Task<bool> QueueTrialEndedAsync(string to, string workspaceName, string planName)
    {
        return QueueAsync(to, TrackerEmailTemplates.TrialEnded(workspaceName, planName, Links.Billing()));
    }

    /// <summary>Whether this deployment delivers email (an SMTP server is configured); false when that cannot be told.</summary>
    public virtual async Task<bool> CanDeliverAsync()
    {
        try
        {
            return await _deliveryChecker.CanDeliverAsync();
        }
        catch (Exception exception)
        {
            Logger.LogWarning(exception, "Could not read the email configuration; no email is sent.");
            return false;
        }
    }

    protected virtual async Task<bool> QueueAsync(string to, EmailContent content)
    {
        if (string.IsNullOrWhiteSpace(to))
        {
            return false;
        }

        if (!await CanDeliverAsync())
        {
            Logger.LogWarning(
                "Email '{Subject}' to {Recipient} was not sent: no SMTP server is configured (set {HostKey}).",
                content.Subject, EmailAddressMask.Mask(to), TrackerSmtpEmailSenderConfiguration.HostConfigurationKey);
            return false;
        }

        try
        {
            await _emailSender.QueueAsync(to.Trim(), content.Subject, content.HtmlBody, isBodyHtml: true);
            return true;
        }
        catch (Exception exception)
        {
            Logger.LogWarning(exception, "Could not queue the email '{Subject}' to {Recipient}.",
                content.Subject, EmailAddressMask.Mask(to));
            return false;
        }
    }

    private Task<bool> NoLinkAsync(string kind)
    {
        Logger.LogWarning(
            "The {Kind} email was not sent: the web app's address is unknown (set {ClientUrlKey}).",
            kind, WebAppLinks.ClientUrlKey);
        return Task.FromResult(false);
    }
}
