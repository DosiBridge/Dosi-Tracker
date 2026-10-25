using System;
using System.Net.Mail;
using System.Threading.Tasks;
using Microsoft.Extensions.Logging;
using Volo.Abp.BackgroundJobs;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Emailing;
using Volo.Abp.Emailing.Smtp;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// The application's email sender: ABP's SMTP sender, sending only once SMTP is configured
/// (<see cref="IEmailDeliveryChecker"/>, see <see cref="TrackerSmtpEmailSenderConfiguration"/>). Until then it sends
/// nothing and logs that it did not — the recipient masked and never the body, which can carry a sign-in link — so a
/// deployment without SMTP keeps working. <see cref="EmailSenderBase.QueueAsync(string, string, string, bool, AdditionalEmailSendingArgs)"/>
/// hands the email to ABP's background email job, which retries a failed delivery with a growing delay; nothing is
/// queued while SMTP is not configured.
/// </summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(typeof(IEmailSender), typeof(ISmtpEmailSender), typeof(TrackerEmailSender))]
public class TrackerEmailSender : SmtpEmailSender
{
    private readonly IEmailDeliveryChecker _deliveryChecker;

    public TrackerEmailSender(
        ICurrentTenant currentTenant,
        ISmtpEmailSenderConfiguration smtpConfiguration,
        IBackgroundJobManager backgroundJobManager,
        IEmailDeliveryChecker deliveryChecker)
        : base(currentTenant, smtpConfiguration, backgroundJobManager)
    {
        _deliveryChecker = deliveryChecker;
    }

    public override async Task QueueAsync(
        string to, string subject, string body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null)
    {
        if (await IsSmtpConfiguredAsync())
        {
            await base.QueueAsync(to, subject, body, isBodyHtml, additionalEmailSendingArgs);
            return;
        }

        LogNotSent(to, subject);
    }

    public override async Task QueueAsync(
        string from, string to, string subject, string body, bool isBodyHtml = true, AdditionalEmailSendingArgs? additionalEmailSendingArgs = null)
    {
        if (await IsSmtpConfiguredAsync())
        {
            await base.QueueAsync(from, to, subject, body, isBodyHtml, additionalEmailSendingArgs);
            return;
        }

        LogNotSent(to, subject);
    }

    public override async Task SendAsync(MailMessage mail, bool normalize = true)
    {
        if (await IsSmtpConfiguredAsync())
        {
            await base.SendAsync(mail, normalize);
            return;
        }

        LogNotSent(mail.To.ToString(), mail.Subject);
    }

    /// <summary>Whether SMTP is configured; a failure to tell (the setting store is unreachable) counts as not.</summary>
    protected virtual async Task<bool> IsSmtpConfiguredAsync()
    {
        try
        {
            return await _deliveryChecker.CanDeliverAsync();
        }
        catch (Exception exception)
        {
            Logger.LogWarning(exception, "Could not read the SMTP configuration; the email is not sent.");
            return false;
        }
    }

    private void LogNotSent(string to, string? subject)
    {
        Logger.LogWarning(
            "Email '{Subject}' to {Recipient} was not sent: no SMTP server is configured (set {HostKey}).",
            subject, EmailAddressMask.Mask(to), TrackerSmtpEmailSenderConfiguration.HostConfigurationKey);
    }
}

/// <summary>Email addresses as they may appear in logs: the first character of the mailbox, then the domain.</summary>
public static class EmailAddressMask
{
    public static string Mask(string? address)
    {
        if (string.IsNullOrWhiteSpace(address))
        {
            return "(none)";
        }

        var trimmed = address.Trim();
        var at = trimmed.LastIndexOf('@');
        return at <= 0 ? "***" : trimmed[0] + "***" + trimmed[at..];
    }
}
